//! The standing proptest gate over the append-only partition-grain
//! `ModelRecipe` pool
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3),
//! plus the fact+mutable-dimension mixed pool (Phase 4).

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::contract::{oracle_obligation, ContractPoint, OracleObligation};
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::{Corner, MutationProfile, SourceFacts, Technique, Trigger};
use smelt_maintenance_testkit::feed::{self, FeedSourcePosture};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::{
    except_all_row_count_via_backend, multiset_equal_via_backend,
};
use smelt_maintenance_testkit::oracle_modes::{
    keyed_end_state_with_retained_departed_keys, KeyedOracleRow, OracleMode,
};
use smelt_maintenance_testkit::recipe::{
    arb_composed_route, arb_composed_route3_schedule, arb_enrichment_edge_recipe,
    arb_enrichment_edge_schedule, arb_keyed_combiner, arb_keyed_schedule, arb_recipe,
    ComposedKeyedRecipe, ComposedRoute, ComposedRoute3Schedule, ConstructKind, EnrichmentJoinKind,
    KeyShape, KeyedCombiner, KeyedRecipe, KeyedSchedule, ModelEdit, ModelRecipe,
    MutableEnrichedRecipe, RecipePool, SourcePosture, SourceRecipe, ValueEnrichedRecipe,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_mixed_schedule, arb_schedule_for, boundary_rows_for,
    check_profile as check_mixed_schedule_profile, read_source_snapshot, scan_clamp_for,
    ConformanceSchedule, ConformanceStep, GenRow, MixedSchedule, MixedStep, StateResidencyOp,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::check_runner::batches_to_rows;
use smelt_runtime::maintenance_driver::{
    driving_steps, resolve_live_membership_recompute_cell, run_windowed_keyed_maintenance,
};

/// A retry policy that never retries — this conformance gate drives a real
/// DuckDB backend directly rather than going through `execute_project`, so
/// there is no `ExecuteRequest`/run reporter to derive one from
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6). `retry_max: 0`
/// keeps every call site's behaviour identical to before retry coverage was
/// extended to these maintenance-driver entry points.
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "maintenance-conformance-gate",
        model_name: "maintenance-conformance-gate",
        reporter: &NO_OP_REPORTER,
    }
}

/// Default deterministic case count for
/// `append_only_partition_pool_upholds_equivalence` — small enough to stay
/// on par with `property_discovery`'s per-target budget (plan Phase 3
/// Implementation shape: "each case ≈ 1-3s … 12 cases keeps the target on
/// par").
const DEFAULT_CASES: usize = 12;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Delete `.smelt/` from `project`'s directory in place — the
/// [`ConformanceStep::DropStateDir`]/[`StateResidencyOp::DropStateDir`]
/// implementation, exposed separately (rather than inlined in the match arm)
/// so `state_deletion.rs`'s anti-vacuity test
/// (`drop_state_dir_step_actually_removes_the_directory`) can call it
/// directly and assert it actually removes the directory — an unobservable
/// no-op here would let the whole residency leg pass while proving nothing
/// (`docs/outcomes/20260816-state-residency/phases/08-plan.md` test 1).
pub fn drop_state_dir(project: &LinkCProject) -> anyhow::Result<()> {
    let dir = project.project_dir.join(".smelt");
    std::fs::remove_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("remove_dir_all({}): {e}", dir.display()))
}

/// Stage `recipe` into a fresh temp project + DuckDB file, returning the
/// loaded [`LinkCProject`].
pub fn stage_recipe(recipe: &ModelRecipe, tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage(recipe, &project_dir, &db_path)
}

/// Routed through [`smelt_backend::Backend::execute_sql`] rather than a raw
/// `duckdb::Connection`
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2) — same
/// `INSERT` text, only the execution channel moved (this helper is private
/// to this file, so the conversion has no blast radius on the sibling test
/// files in this directory that also call into `gate.rs`'s *public*
/// surface).
async fn insert_row(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    row: &smelt_maintenance_testkit::schedule_gen::GenRow,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    backend
        .execute_sql(&format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert row: {e}"))?;
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` through the real
/// `execute_project` pipeline, asserting S-restricted multiset equivalence
/// after every `RunWindow` step (design §6). Returns the fully-populated
/// [`STracker`] plus the index of the last recorded run, so
/// `harness_self_check` can reuse the green end-state without re-driving
/// the whole schedule.
pub async fn drive_and_assert(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
) -> anyhow::Result<(STracker, usize)> {
    // Local, reassignable handle: `ConformanceStep::FreshClone` replaces it
    // with a clone at a new path mid-loop
    // (`docs/outcomes/20260816-state-residency/phases/08-plan.md` task 3).
    // Every subsequent reference to `project` below therefore already reads
    // this shadowed local, not the caller's original handle.
    let mut project = project.clone();
    let mut tracker = STracker::new(&recipe.source);
    let mut last_k: Option<usize> = None;
    // The most recent `RewriteModel` step's edit (Phase 9), or `None` before
    // any rewrite — threads into every subsequent assertion so the oracle
    // re-renders against the CURRENT on-disk body, never the pre-rewrite one
    // (plan Phase 9 review checklist).
    let mut current_edit: Option<ModelEdit> = None;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            ConformanceStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_row(&project, recipe, row).await?;
                }

                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(&project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::AppendLateRow(row) => {
                insert_row(&project, recipe, row).await?;
            }
            ConformanceStep::RerunWindow { start, end } => {
                // Redelivery: same window as an earlier `RunWindow`, no new
                // rows. Never-fold-twice under the partition-grain
                // DELETE+INSERT full-replace technique must hold (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(&project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::FullRefreshRun => {
                // Unwindowed run: `execute_project` takes the full-refresh
                // arm (drop + rebuild from the CURRENT full source
                // contents) whenever no `start`/`end` is supplied (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.full_refresh = true;
                request.start = None;
                request.end = None;
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_full_refresh(snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(&project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::BackfillRegion { start, end } => {
                // An explicit backfill: same execution shape as `RunWindow`
                // with no accompanying insert (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(&project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::RewriteModel { edit } => {
                // Definition change (Phase 9): rewrite the model file on
                // disk with `edit` applied. No run happens here — the next
                // *Window step re-discovers the model from disk (already
                // the harness's behaviour) and compiles/executes whatever
                // SQL is now on disk.
                let model_path = project
                    .project_dir
                    .join(format!("models/{}.sql", recipe.model_name));
                std::fs::write(
                    &model_path,
                    render::render_model_file_with_edit(recipe, *edit),
                )?;
                current_edit = Some(*edit);
            }
            ConformanceStep::DropStateDir => {
                // Residency leg (`docs/outcomes/20260816-state-residency/
                // phases/08-plan.md` task 3): delete `.smelt/` in place. No
                // accompanying run — the next window step's own equivalence
                // assertion is the proof that nothing correctness-class rode
                // on the deleted directory.
                drop_state_dir(&project)?;
            }
            ConformanceStep::FreshClone => {
                // Residency leg: replace the project handle with a fresh
                // clone at a new path, carrying no `.smelt/` — catches
                // anything keyed on the old absolute path that a same-path
                // `DropStateDir` cannot.
                let dest = project
                    .project_dir
                    .parent()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "project_dir {:?} has no parent to clone alongside",
                            project.project_dir
                        )
                    })?
                    .join(format!("fresh_clone_{i}"));
                project = project.fresh_clone(&dest)?;
            }
        }
    }

    let last_k =
        last_k.ok_or_else(|| anyhow::anyhow!("schedule {schedule:?} had no RunWindow step"))?;
    Ok((tracker, last_k))
}

/// The S-restricted oracle assertion (design §6): materialize `S_k`, then
/// `EXCEPT ALL`-compare it (both directions) against the maintained output
/// table `main.<model_name>`.
pub async fn assert_equivalence(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    assert_equivalence_at_point(project, recipe, tracker, k, &ContractPoint::Default).await
}

/// [`assert_equivalence`] parameterised by contract-lattice point
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`):
/// dispatches on `smelt_logical::contract::oracle_obligation(point)` rather
/// than re-deriving a per-point comparator — [`OracleObligation::Exact`]
/// and [`OracleObligation::ExactOverRestrictedS`] are the same both-
/// direction `multiset_equal_via_backend` check
/// [`assert_equivalence_with_edit`] already performs, evaluated against
/// `tracker.s_at_for_point(k, point)` instead of the unrestricted `s_at(k)`;
/// [`OracleObligation::Bracketed`] (`deferral`) asserts
/// `full_refresh(S_settled) ⊆ maintained ⊆ full_refresh(S)`, one `EXCEPT
/// ALL` direction per leg. `input_frontier` is only consulted for the
/// bracketed obligation — every other point ignores it.
pub async fn assert_equivalence_at_point(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    point: &ContractPoint,
) -> anyhow::Result<()> {
    assert_equivalence_at_point_with_frontier(project, recipe, tracker, k, point, None).await
}

/// [`assert_equivalence_at_point`] with an explicit `input_frontier`
/// (days-from-CE) for the [`OracleObligation::Bracketed`] leg — required
/// whenever `point` is [`ContractPoint::Deferral`]; ignored otherwise.
pub async fn assert_equivalence_at_point_with_frontier(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    point: &ContractPoint,
    input_frontier: Option<i64>,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);

    match oracle_obligation(point) {
        OracleObligation::Exact | OracleObligation::ExactOverRestrictedS => {
            tracker
                .materialize_s_for_point(backend.as_ref(), k, point)
                .await?;
            let oracle_sql = tracker.s_restricted_oracle_sql(recipe);
            let equal =
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
            if !equal {
                anyhow::bail!(
                    "equivalence violated for model {:?} at run {k} under point {point:?}: \
                     maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
                    recipe.model_name
                );
            }
            Ok(())
        }
        OracleObligation::Bracketed => {
            let input_frontier = input_frontier.ok_or_else(|| {
                anyhow::anyhow!(
                    "point {point:?} has a Bracketed oracle obligation but no input_frontier \
                     was supplied"
                )
            })?;

            // Leg 1: maintained ⊆ full_refresh(S).
            tracker
                .materialize_s_for_point(backend.as_ref(), k, point)
                .await?;
            let full_oracle_sql = tracker.s_restricted_oracle_sql(recipe);
            let maintained_minus_full = except_all_row_count_via_backend(
                backend.as_ref(),
                &maintained_sql,
                &full_oracle_sql,
            )
            .await?;
            if maintained_minus_full != 0 {
                anyhow::bail!(
                    "bracketed equivalence violated for model {:?} at run {k}: maintained has \
                     {maintained_minus_full} row(s) not in full_refresh(S) ({full_oracle_sql:?})",
                    recipe.model_name
                );
            }

            // Leg 2: full_refresh(S_settled) ⊆ maintained.
            tracker
                .materialize_s_settled(backend.as_ref(), k, point, input_frontier)
                .await?;
            let settled_oracle_sql = tracker.s_restricted_oracle_sql(recipe);
            let settled_minus_maintained = except_all_row_count_via_backend(
                backend.as_ref(),
                &settled_oracle_sql,
                &maintained_sql,
            )
            .await?;
            if settled_minus_maintained != 0 {
                anyhow::bail!(
                    "bracketed equivalence violated for model {:?} at run {k}: \
                     full_refresh(S_settled) ({settled_oracle_sql:?}) has \
                     {settled_minus_maintained} row(s) not in maintained",
                    recipe.model_name
                );
            }
            Ok(())
        }
    }
}

/// [`assert_equivalence`] generalised over an optional post-`RewriteModel`
/// [`ModelEdit`] (Phase 9): when `edit` is `Some`, the oracle re-renders
/// against the REWRITTEN body
/// (`STracker::s_restricted_oracle_sql_with_edit`) rather than the
/// original recipe's own body — never comparing the old body against the
/// new output (plan Phase 9 review checklist). `edit: None` reproduces
/// [`assert_equivalence`]'s exact pre-Phase-9 behaviour.
pub async fn assert_equivalence_with_edit(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    edit: Option<ModelEdit>,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = match edit {
        Some(edit) => tracker.s_restricted_oracle_sql_with_edit(recipe, edit),
        None => tracker.s_restricted_oracle_sql(recipe),
    };
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "S-restricted equivalence violated for model {:?} at run {k} (edit {edit:?}): \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// `append_only_partition_pool_upholds_equivalence` (plan Phase 3 TDD
/// list): the standing proptest gate — deterministic seed, small N (default
/// [`DEFAULT_CASES`], `SMELT_CONFORMANCE_CASES` env override). Each case:
/// generate recipe → classify → if admitted, generate schedule → drive real
/// `execute_project` per step → assert S-restricted multiset equivalence
/// after every run step.
#[test]
fn append_only_partition_pool_upholds_equivalence() {
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
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

        match verdict {
            Verdict::Refused(_) => continue,
            Verdict::Admitted(_) => {
                admitted_cases += 1;
                rt.block_on(drive_and_assert(&project, &recipe, &schedule))
                    .unwrap_or_else(|e| {
                        panic!(
                            "case {i}: recipe {recipe:?} schedule {schedule:?} \
                             equivalence check failed: {e}"
                        )
                    });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
}

/// `admission_rate_stays_above_floor` (plan Phase 3 TDD list): generator
/// health — over the deterministic sample, at least 40% of (non-adversarial)
/// recipes admit at least one cell.
#[test]
fn admission_rate_stays_above_floor() {
    const N: usize = 50;
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let mut admitted = 0;
    for i in 0..N {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));
        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));
        if matches!(verdict, Verdict::Admitted(_)) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.40,
        "admission rate {rate:.2} over N={N} fell below the 40% generator-health floor \
         ({admitted}/{N} admitted)"
    );
}

// ---------------------------------------------------------------------
// Phase 4: the fact+mutable-dimension mixed pool (`MutableEnrichedRecipe`).
// ---------------------------------------------------------------------

/// Default deterministic case count for `mutable_pool_settles_to_full_refresh`
/// — smaller than [`DEFAULT_CASES`] since each case also drives a dimension
/// mutation + catch-up run on top of the fact-window runs.
const MIXED_DEFAULT_CASES: usize = 6;

fn mixed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_MIXED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIXED_DEFAULT_CASES)
}

/// Generous upper bound on distinct fact/dimension ids a generated
/// [`MixedSchedule`] can ever produce (`arb_mixed_schedule`'s shape: at most
/// 3 windows × 2 rows), so [`stage_mixed_recipe`] can pre-seed every
/// dimension row a schedule might reference (an unmatched fact id would
/// silently vanish from the INNER JOIN output, which is a staging bug, not a
/// case the pool ever intends to reach).
const MIXED_DIM_SEED_MAX_ID: i64 = 12;

/// Stage a [`MutableEnrichedRecipe`] into a fresh temp project + DuckDB file:
/// writes both source YAMLs + the model file, creates both physical source
/// tables, and pre-seeds the dimension table with one row per id in
/// `1..=MIXED_DIM_SEED_MAX_ID` (`attr = id * 100`) so every fact row a
/// generated schedule inserts already has a matching dimension row to join
/// against.
pub fn stage_mixed_recipe(
    recipe: &MutableEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    for id in 1..=MIXED_DIM_SEED_MAX_ID {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one fact row into `recipe`'s staged fact source table.
pub fn insert_fact_row(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.fact.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`MutableEnrichedRecipe`] through the real maintenance
/// derivation — the mixed-pool counterpart of `verdict::classify`, kept here
/// rather than in `verdict.rs` (outside this phase's edit scope, plan
/// Critical files) since `verdict::classify` only accepts a `ModelRecipe`.
pub fn classify_mixed(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged mixed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let plan_result =
        smelt_db::maintenance_plan_report(&db, workspace, target).ok_or_else(|| {
            anyhow::anyhow!(
                "no maintenance plan report for mixed-pool model {:?}",
                recipe.model_name
            )
        })?;
    Ok(plan_result.plan)
}

/// The window a `MutateDimension` step's `id` was produced in, by scanning
/// the schedule's own `RunWindow` steps — the same lookup rule
/// `schedule_gen::check_profile` uses.
fn fact_window_for_id(
    schedule: &MixedSchedule,
    id: i64,
) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
    schedule.0.iter().find_map(|s| match s {
        MixedStep::RunWindow { start, end, rows } => {
            rows.iter().any(|r| r.id == id).then_some((*start, *end))
        }
        _ => None,
    })
}

/// The settled-point oracle assertion (design §6 "Mixed models"): the fact
/// side reads the S-restricted temp table (`tracker`'s `S_k`); the
/// dimension side always reads its CURRENT physical state.
async fn assert_mixed_settled(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    // `STracker::materialize_s` names its temp table `oracle_<source_name>`
    // (an internal convention of `s_tracker.rs`, mirrored here rather than
    // exposed as a public accessor since this is the only Phase 4 call site
    // needing it).
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "settled-point equivalence violated for {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` (a [`MutableEnrichedRecipe`])
/// through the real `execute_project` pipeline: every `RunWindow` step
/// inserts its rows then runs; every `MutateDimension` step mutates the
/// dimension in place and records the affected window as outstanding
/// (`STracker::record_dimension_mutation`). Equivalence is asserted after
/// every `RunWindow` step ONLY while the tracker reports
/// [`OracleMode::SRestricted`] — the weaker expected-staleness contract
/// while [`OracleMode::SettledPoint`] holds is recorded non-fatally, never
/// asserted as a hard failure (design §6 "Mixed models").
async fn drive_mixed_and_assert(
    project: &LinkCProject,
    recipe: &MutableEnrichedRecipe,
    schedule: &MixedSchedule,
) -> anyhow::Result<()> {
    let mut tracker = STracker::new(&recipe.fact);
    let mut settled_assertions = 0usize;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            MixedStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_fact_row(project, recipe, row)?;
                }

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project
                    .run_quiet(&format!("mixed-run-{i}"), request)
                    .await?;

                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.fact)
                };
                let k = tracker.record_run(*start, *end, snapshot);

                match tracker.oracle_mode() {
                    OracleMode::SRestricted => {
                        assert_mixed_settled(project, recipe, &tracker, k).await?;
                        settled_assertions += 1;
                    }
                    OracleMode::SettledPoint => {
                        // Expected staleness: some window's dimension
                        // mutation has not yet been caught up — non-fatal
                        // (design §6).
                    }
                }
            }
            MixedStep::MutateDimension { id, new_attr } => {
                let conn = project.connect()?;
                conn.execute(
                    &format!(
                        "UPDATE main.sources_{} SET {} = {} WHERE {} = {}",
                        recipe.dimension.name,
                        recipe.dimension.payload_column,
                        new_attr,
                        recipe.dimension.key_column,
                        id,
                    ),
                    [],
                )?;
                drop(conn);

                let window = fact_window_for_id(schedule, *id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "MutateDimension targets id {id} which no RunWindow step produced \
                         (schedule_gen::check_profile should have caught this)"
                    )
                })?;
                tracker.record_dimension_mutation(window);
            }
        }
    }

    anyhow::ensure!(
        settled_assertions > 0,
        "schedule {schedule:?} never reached a settled point — no assertion ran"
    );
    Ok(())
}

/// `mutable_pool_settles_to_full_refresh` (plan Phase 4 TDD list):
/// fact+mutable-dimension recipes (the `daily_events_enriched` shape,
/// generated schedules) hold full equivalence at every settled point;
/// expected-staleness in between is recorded, never fatal.
#[test]
fn mutable_pool_settles_to_full_refresh() {
    let n = mixed_case_count();
    let mut runner = TestRunner::deterministic();
    let recipe = MutableEnrichedRecipe::new();
    let schedule_strat = arb_mixed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        check_mixed_schedule_profile(&schedule).unwrap_or_else(|e| {
            panic!("case {i}: generated mixed schedule {schedule:?} failed its own self-check: {e}")
        });

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage mixed recipe: {e}"));

        let plan = classify_mixed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify_mixed failed: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "case {i}: mixed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression (expected at least the UpstreamMutation ColumnScopedMerge cell)"
        );

        rt.block_on(drive_mixed_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!("case {i}: mixed schedule {schedule:?} equivalence check failed: {e}")
            });
    }
}

// ---------------------------------------------------------------------
// Phase 5: the `grain: key` pool (`KeyedRecipe`).
// ---------------------------------------------------------------------

/// Default deterministic case count for `keyed_pool_upholds_end_state_equivalence`
/// — small since each case drives several `execute_project` windows.
const KEYED_DEFAULT_CASES: usize = 6;

fn keyed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_KEYED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(KEYED_DEFAULT_CASES)
}

/// Stage a [`KeyedRecipe`] into a fresh temp project + DuckDB file.
pub fn stage_keyed_recipe(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed(recipe, &project_dir, &db_path)
}

/// [`stage_keyed_recipe`], additionally staging a downstream `SELECT * FROM
/// smelt.<model>` consumer model (`render::stage_keyed_with_downstream`,
/// phase 8 task 5) — opt-in so every pre-existing keyed recipe's staged
/// project shape stays byte-identical.
pub fn stage_keyed_recipe_with_downstream(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_keyed_with_downstream(recipe, &project_dir, &db_path)
}

/// Stage a [`KeyedRecipe`] built over
/// [`SourceRecipe::unclocked_append_only_dimension`] — [`stage_keyed_recipe`]
/// (via `render::stage_keyed`) always emits its `AppendOnly` source's
/// standard `source_yaml()`, which unconditionally declares a `timeseries:`
/// block; this probe needs an `AppendOnly`-postured source with NO
/// `timeseries:` block anywhere in the model (the model/classifier
/// plan-agreement finding, `docs/plans/20260809-keyed-frontier.md` Phase 3
/// review), so it writes a bespoke source YAML directly — same physical
/// `(d DATE, id INTEGER, val INTEGER)` shape `stage_keyed`'s `AppendOnly`
/// DDL branch already expects (mirrors `stage_mixed_recipe`'s own
/// bespoke-YAML staging above for the same reason).
fn stage_keyed_unclocked_append_only(
    recipe: &KeyedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        render::render_keyed_model_file(recipe),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        format!(
            "description: plan/classifier-agreement probe source, append-only with no \
             declared timeseries block.\n\
             mutation_profile: append_only\n\
             columns:\n\
             \x20 - name: {d}\n    type: DATE\n\
             \x20 - name: {id}\n    type: INTEGER\n\
             \x20 - name: {val}\n    type: INTEGER\n",
            d = recipe.source.clock_column,
            id = recipe.source.key_column,
            val = recipe.source.payload_column,
        ),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{name} ({d} DATE, {id} INTEGER, {val} INTEGER);",
        name = recipe.source.name,
        d = recipe.source.clock_column,
        id = recipe.source.key_column,
        val = recipe.source.payload_column,
    ))?;
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one row into a [`KeyedRecipe`]'s staged driving-source table.
pub fn insert_row_keyed(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Insert one row into a snapshot-reconcile [`KeyedRecipe`]'s staged
/// (unclocked, `mutable_snapshot`) driving-source table — `(id, attr)`, no
/// clock column (`SourceRecipe::mutable_dimension`'s shape, unlike
/// [`insert_row_keyed`]'s clocked `(d, id, val)`).
fn insert_row_keyed_snapshot(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES ({id}, {attr})",
            recipe.source.name
        ),
        [],
    )?;
    Ok(())
}

/// Update a snapshot-reconcile [`KeyedRecipe`]'s staged dimension row's
/// `attr` column.
fn update_row_keyed_snapshot(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{} SET {} = {attr} WHERE {} = {id}",
            recipe.source.name, recipe.source.payload_column, recipe.source.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Delete a snapshot-reconcile [`KeyedRecipe`]'s staged dimension row — the
/// genuine-departure case: `id` must be RETAINED, unchanged, in the
/// maintained table after the next run (`incremental_shapes.md` §"The two
/// run shapes" — snapshot-reconcile never deletes a departed key).
fn delete_row_keyed_snapshot(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{} WHERE {} = {id}",
            recipe.source.name, recipe.source.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Phase 3 (`docs/plans/20260809-keyed-frontier.md`): drive the ONE family
/// the admission matrix actually admits under snapshot-reconcile
/// (plain-overwrite, `ANY_VALUE`) end to end through the real
/// `execute_project` pipeline and the now-built snapshot-reconcile
/// executor: seed rows, run (creation), mutate/delete/insert source rows,
/// run again (reconcile), and assert the maintained table equals the
/// current snapshot's own aggregation UNION the pre-mutation state of any
/// key that departed the snapshot — the SAME retained-departed-keys
/// carve-out `retained_departed_keys_adjusts_the_oracle` above pins as pure
/// data, now exercised against a real backend.
#[tokio::test]
async fn snapshot_reconcile_plain_overwrite_settles_with_retained_departed_keys() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::PlainOverwrite);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage snapshot-reconcile recipe");

    // Seed three keys.
    insert_row_keyed_snapshot(&project, &recipe, 1, 100).expect("seed id=1");
    insert_row_keyed_snapshot(&project, &recipe, 2, 200).expect("seed id=2");
    insert_row_keyed_snapshot(&project, &recipe, 3, 300).expect("seed id=3");

    // First run: no event-time window (snapshot-reconcile has no clock) —
    // creates the target table.
    project
        .run_quiet("snapshot-reconcile-1", base_request("dev"))
        .await
        .expect("first (creation) run must succeed");

    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let full_scan_oracle_sql = format!(
        "SELECT {key}, ANY_VALUE({attr}) AS current_val FROM main.sources_{name} GROUP BY {key}",
        key = recipe.source.key_column,
        attr = recipe.source.payload_column,
        name = recipe.source.name,
    );
    {
        let backend = project.backend().await.expect("backend");
        let equal =
            multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &full_scan_oracle_sql)
                .await
                .expect("comparison must run");
        assert!(equal, "creation run must equal the full-scan oracle");
    }

    // Snapshot the pre-mutation source state — the retained-departed-keys
    // formula needs the departing key's value AS OF BEFORE it departed.
    {
        let conn = project.connect().expect("connect");
        conn.execute_batch(&format!(
            "CREATE TABLE main.pre_mutation_snapshot AS SELECT * FROM main.sources_{}",
            recipe.source.name
        ))
        .expect("snapshot pre-mutation state");
    }

    // Mutate: update id=1's value, delete id=2 (genuine departure), insert
    // a fresh id=4.
    update_row_keyed_snapshot(&project, &recipe, 1, 999).expect("update id=1");
    delete_row_keyed_snapshot(&project, &recipe, 2).expect("delete id=2");
    insert_row_keyed_snapshot(&project, &recipe, 4, 400).expect("insert id=4");

    // Second run: still no window — reconciles via the whole-source MERGE.
    project
        .run_quiet("snapshot-reconcile-2", base_request("dev"))
        .await
        .expect("second (reconcile) run must succeed");

    let adjusted_oracle_sql = format!(
        "{full_scan_oracle_sql} \
         UNION ALL \
         SELECT {key}, {attr} AS current_val FROM main.pre_mutation_snapshot \
         WHERE {key} NOT IN (SELECT {key} FROM main.sources_{name})",
        key = recipe.source.key_column,
        attr = recipe.source.payload_column,
        name = recipe.source.name,
    );
    {
        let backend = project.backend().await.expect("backend");
        let equal =
            multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &adjusted_oracle_sql)
                .await
                .expect("comparison must run");
        assert!(
            equal,
            "reconcile run must equal the oracle's current rows plus the retained departed key"
        );
    }

    // Explicit assertion, not just the multiset comparison: the departed
    // key (id=2) is present, unchanged from its PRE-mutation value (200) —
    // not silently deleted.
    let conn = project.connect().expect("connect");
    let departed_value: i64 = conn
        .query_row(
            &format!(
                "SELECT current_val FROM main.{} WHERE id = 2",
                recipe.model_name
            ),
            [],
            |row| row.get(0),
        )
        .expect("departed key must still be present");
    assert_eq!(
        departed_value, 200,
        "the departed key must be RETAINED at its pre-departure value, never deleted"
    );
}

/// Phase 3: `--event-time-start`/`--event-time-end` on a snapshot-reconcile
/// model (no clocked driving source) is rejected loudly, naming the run
/// shape — rather than silently ignored or dispatched through the
/// window-forward executor.
#[tokio::test]
async fn snapshot_reconcile_rejects_event_time_window() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::PlainOverwrite);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage snapshot-reconcile recipe");
    insert_row_keyed_snapshot(&project, &recipe, 1, 100).expect("seed id=1");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());

    let err = project
        .run_quiet("snapshot-reconcile-windowed", request)
        .await
        .expect_err("an event-time window on a snapshot-reconcile model must be refused");
    let message = format!("{err:#}");
    assert!(
        message.contains("snapshot-reconcile"),
        "expected the refusal to name the snapshot-reconcile run shape: {message}"
    );
}

/// Classify a staged [`KeyedRecipe`] through the real maintenance derivation
/// — the keyed-pool counterpart of `classify`/`classify_mixed`, kept here
/// rather than in `verdict.rs` (outside this phase's edit scope, plan
/// Critical files). Returns the derived plan (possibly with zero cells) plus
/// every diagnostic on the target model, so a refusal case can name the
/// diagnostic that backs it.
pub fn classify_keyed_full(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged keyed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Classify a staged [`KeyedRecipe`], requiring an admitted (non-empty) plan
/// — the happy-path wrapper around [`classify_keyed_full`] for cases that
/// expect admission.
pub fn classify_keyed(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
    let (plan, diags) = classify_keyed_full(project, recipe)?;
    match plan {
        Some(plan) if !plan.cells.is_empty() => Ok(plan),
        _ => anyhow::bail!(
            "keyed recipe {:?} admitted no cells: diagnostics={diags:#?}",
            recipe.model_name
        ),
    }
}

/// The maintained table's PRESENTED columns only, as `(name, data_type)`
/// pairs in physical column order — excludes any physical column whose name
/// contains the reserved `__` decomposed-state marker
/// (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in keyed
/// models"). A state-bearing model's physical table carries its hidden
/// state columns alongside the presented ones (`MAX_BY`/`MIN_BY`, row 5);
/// a bare `SELECT *` against the live table — unlike a `ref()`-mediated
/// read through smelt's own compiler, which the `presentation_projection`
/// mechanism already rewrites — would leak them into an oracle comparison
/// with a different column count. Column names/types/order come off
/// `information_schema.columns`, mirroring `probes::read_full_output_as_text`.
fn presented_columns_with_types(project: &LinkCProject, model_name: &str) -> Vec<(String, String)> {
    let conn = project
        .connect()
        .expect("connect for presented-column listing");
    let columns: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = 'main' AND table_name = '{model_name}' \
                 AND column_name NOT LIKE '%\\_\\_%' ESCAPE '\\' \
                 ORDER BY ordinal_position",
            ))
            .expect("prepare presented-column listing");
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query presented-column listing")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect presented-column listing")
    };
    assert!(
        !columns.is_empty(),
        "model {model_name:?} reported zero presented columns via information_schema — \
         staging bug or an over-eager state-column filter"
    );
    columns
}

/// EVERY physical column name of `model_name`'s maintained table, in
/// ordinal order — unlike [`presented_columns_with_types`], applies no `__`
/// filter. Used both to prove a state-bearing recipe's table really does
/// carry hidden state columns (a vacuity guard,
/// `state_bearing_recipes_physically_carry_state_columns`) and to prove a
/// downstream `ref()`-mediated consumer carries none
/// (`assert_downstream_hides_state`).
fn all_physical_column_names(project: &LinkCProject, model_name: &str) -> Vec<String> {
    let conn = project
        .connect()
        .expect("connect for physical-column listing");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = '{model_name}' \
             ORDER BY ordinal_position",
        ))
        .expect("prepare physical-column listing");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query physical-column listing")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect physical-column listing")
}

/// `(name, data_type)` pairs (from [`presented_columns_with_types`]) into a
/// float-aware select-list fragment: `DOUBLE`/`FLOAT`/`REAL` columns are
/// wrapped `ROUND(col, 6) AS col`, every other column is selected bare. Used
/// to build BOTH sides of a keyed end-state comparison from the exact same
/// column list, so a maintained/oracle pair only ever disagrees in the
/// column list itself (a real bug) rather than in which side got rounded.
///
/// Float-aware, not exact, because DuckDB's own `STDDEV_SAMP` uses a
/// numerically stable (Welford-style) accumulation pass while the
/// decomposed `(n, Σx, Σx²)` state this outcome derives recomputes variance
/// from the raw sums (`incremental_shapes.md` §"Decomposed state (rung 2) in
/// keyed models") — the two agree only to floating-point noise (~1e-12),
/// so an exact `EXCEPT ALL` would flake. [`harness_self_check`]'s
/// `float_equivalence_comparison_tolerates_last_bit_only` pins this
/// tolerance so it cannot silently widen into swallowing a real fold bug.
fn rounded_select_list(columns: &[(String, String)]) -> String {
    columns
        .iter()
        .map(|(name, data_type)| {
            if matches!(data_type.as_str(), "DOUBLE" | "FLOAT" | "REAL") {
                format!("ROUND({name}, 6) AS {name}")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The end-state equivalence assertion for a [`KeyedRecipe`] (design §6
/// "Keyed-grain carve-outs"; `incremental_shapes.md` §"End-state equivalence"):
/// materialize `S_k` (the union, across every run so far, of that run's own
/// window's rows — exactly [`STracker::s_at`]'s definition, which coincides
/// with "every delta row a window-forward keyed run has folded so far" since
/// a keyed run merges every row landing in its own window and no
/// re-delivery occurs in a generated [`KeyedSchedule`]), then compare the
/// maintained table's full contents against the recipe's own body evaluated
/// over `S_k`. Both sides are selected through the same float-aware,
/// presented-columns-only projection ([`rounded_select_list`]) built from
/// one `information_schema`-derived column list.
pub async fn assert_keyed_equivalence(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let columns = presented_columns_with_types(project, &recipe.model_name);
    let select_list = rounded_select_list(&columns);
    let maintained_sql = format!("SELECT {select_list} FROM main.{}", recipe.model_name);
    let oracle_body =
        render::render_keyed_oracle_body_over(recipe, &format!("oracle_{}", recipe.source.name));
    let oracle_sql = format!("SELECT {select_list} FROM ({oracle_body}) AS oracle_sub");
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "keyed end-state equivalence violated for model {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Phase 8 task 5: asserts a staged downstream `SELECT * FROM
/// smelt.<model_name>` consumer (`stage_keyed_recipe_with_downstream`,
/// model file `<model_name>_downstream.sql`) materializes with EXACTLY the
/// upstream's presented columns (no `__`-marked names — `presentation_projection`
/// rewrites the wildcard at compile time, `incremental_shapes.md` §"Decomposed
/// state (rung 2) in keyed models") and multiset-equals the upstream's
/// presented contents — the end-to-end DuckDB witness for the hiding
/// mechanism (unit-tested at compile time in row 4) proven against a real
/// run. Float-aware via the same [`rounded_select_list`]
/// [`assert_keyed_equivalence`] uses.
async fn assert_downstream_hides_state(project: &LinkCProject, model_name: &str) {
    let downstream_name = format!("{model_name}_downstream");

    let downstream_physical_columns = all_physical_column_names(project, &downstream_name);
    let leaked: Vec<_> = downstream_physical_columns
        .iter()
        .filter(|c| c.contains("__"))
        .collect();
    assert!(
        leaked.is_empty(),
        "downstream consumer {downstream_name:?} carries `__`-marked physical column(s) \
         {leaked:?} — presentation_projection failed to hide upstream state from a \
         ref()-mediated read"
    );

    let upstream_columns = presented_columns_with_types(project, model_name);
    let upstream_names: Vec<&String> = upstream_columns.iter().map(|(n, _)| n).collect();
    let downstream_names: Vec<&String> = downstream_physical_columns.iter().collect();
    assert_eq!(
        upstream_names, downstream_names,
        "downstream consumer {downstream_name:?}'s physical column list does not match \
         upstream {model_name:?}'s presented columns"
    );

    let select_list = rounded_select_list(&upstream_columns);
    let upstream_sql = format!("SELECT {select_list} FROM main.{model_name}");
    let downstream_sql = format!("SELECT {select_list} FROM main.{downstream_name}");
    let backend = project
        .backend()
        .await
        .expect("backend for downstream comparison");
    let equal = multiset_equal_via_backend(backend.as_ref(), &upstream_sql, &downstream_sql)
        .await
        .expect("compare downstream consumer to upstream presented contents");
    assert!(
        equal,
        "downstream consumer {downstream_name:?} does not multiset-equal upstream \
         {model_name:?}'s presented contents: upstream ({upstream_sql:?}) != downstream \
         ({downstream_sql:?})"
    );
}

/// Drive `schedule` against `project`/`recipe` (a [`KeyedRecipe`] under the
/// window-forward posture) through the real `execute_project` pipeline,
/// asserting end-state equivalence after every window.
pub async fn drive_keyed_and_assert(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    drive_keyed_and_assert_with_state_ops(
        project,
        recipe,
        schedule,
        &std::collections::BTreeMap::new(),
    )
    .await
}

/// [`drive_keyed_and_assert`], additionally applying a
/// [`StateResidencyOp`] before window `i` whenever `ops` names one for that
/// index (`docs/outcomes/20260816-state-residency/phases/08-plan.md` task
/// 5) — the keyed pool's residency hook. `KeyedSchedule`/`KeyedRunWindow`
/// themselves stay untouched: no new field, no churn to every existing
/// construction site.
pub async fn drive_keyed_and_assert_with_state_ops(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    schedule: &KeyedSchedule,
    ops: &std::collections::BTreeMap<usize, StateResidencyOp>,
) -> anyhow::Result<()> {
    let mut project = project.clone();
    let mut tracker = STracker::new(&recipe.source);

    for (i, window) in schedule.0.iter().enumerate() {
        if let Some(op) = ops.get(&i) {
            project = match op {
                StateResidencyOp::DropStateDir => {
                    drop_state_dir(&project)?;
                    project
                }
                StateResidencyOp::FreshClone => {
                    let dest = project
                        .project_dir
                        .parent()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "project_dir {:?} has no parent to clone alongside",
                                project.project_dir
                            )
                        })?
                        .join(format!("fresh_clone_keyed_{i}"));
                    project.fresh_clone(&dest)?
                }
            };
        }

        for row in &window.rows {
            insert_row_keyed(&project, recipe, row)?;
        }

        let snapshot = {
            let conn = project.connect()?;
            read_source_snapshot(&conn, &recipe.source)
        };

        let mut request = base_request("dev");
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        project
            .run_quiet(&format!("keyed-run-{i}"), request)
            .await?;

        let k = tracker.record_run(window.start, window.end, snapshot);
        assert_keyed_equivalence(&project, recipe, &tracker, k).await?;
    }
    Ok(())
}

/// `keyed_pool_upholds_end_state_equivalence` (plan Phase 5 TDD list):
/// keyed recipes (additive + idempotent combiner families, key re-touch
/// across windows) equal the oracle's end state at settled points.
#[test]
fn keyed_pool_upholds_end_state_equivalence() {
    let n = keyed_case_count();
    let mut runner = TestRunner::deterministic();
    let combiner_strat = arb_keyed_combiner();
    let schedule_strat = arb_keyed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let combiner = combiner_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedRecipe::new_window_forward(combiner);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} failed to stage: {e}"));

        let plan = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} classify failed: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "case {i}: keyed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression"
        );
        admitted_cases += 1;

        rt.block_on(drive_keyed_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: keyed recipe {recipe:?} schedule {schedule:?} equivalence check \
                     failed: {e}"
                )
            });
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic keyed sample admitted zero cases — generator/derivation regression"
    );
}

/// `retained_departed_keys_adjusts_the_oracle` (plan Phase 5 TDD list):
/// snapshot-reconcile schedules generating deletes compare against oracle
/// rows ∪ retained departed keys (`incremental_shapes.md` §"End-state
/// equivalence"). Two halves: (1) an ADDITIVE-combiner (fold-family) keyed
/// recipe over an unclocked (zero-clocked-driving-source) source still
/// refuses its *targeted* keyed-fold cell fail-loud
/// (`Refusal::NoAdmissibleTechnique`/`Refusal::ScanUnbounded`, named on the
/// plan itself — `maintenance-plan purity`: consumed, not re-derived) —
/// the snapshot-reconcile run shape (`incremental_shapes.md` §"The two run
/// shapes") is supportable now (Phase 3, `docs/plans/20260809-keyed-
/// frontier.md`), but a fold-family column is refused under it per the
/// admission matrix (double-count/observer-semantics reasons) regardless —
/// the universal `Trigger::Backfill`/whole-table-recompute cell every model
/// admits (`incremental_models.md` §"Per-cell admission" — "a recompute is
/// the universal ground-truth reset") stays available as the escape hatch,
/// but no `Trigger::NewData` cell is ever admitted for this source; (2) the
/// pure oracle adjustment that refusal defers to is independently pinned as
/// data (`oracle_modes::keyed_end_state_with_retained_departed_keys`) — the
/// SAME formula [`snapshot_reconcile_plain_overwrite_settles_with_retained_
/// departed_keys`] below exercises end-to-end against the real, now-built
/// executor for the one family the matrix actually admits
/// (plain-overwrite).
#[test]
fn retained_departed_keys_adjusts_the_oracle() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage unclocked keyed recipe");

    let (plan, _diags) =
        classify_keyed_full(&project, &recipe).expect("classify unclocked keyed recipe");
    let plan = plan.expect(
        "maintenance_plan_report must still return a plan (the universal \
         Backfill cell), even when the targeted keyed fold is refused",
    );

    assert!(
        !plan.cells.iter().any(
            |c| matches!(&c.trigger, Trigger::NewData { source } if source == &recipe.source.name)
        ),
        "an unclocked (snapshot-reconcile) keyed model must never admit a targeted NewData \
         fold cell today: {plan:#?}"
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, .. }
                if trigger.contains(&recipe.source.name)
        )),
        "expected a named NoAdmissibleTechnique refusal naming the unclocked driving source, \
         got: {:#?}",
        plan.refusals
    );

    // The pure carve-out formula this refusal defers to.
    let oracle_rows = vec![
        KeyedOracleRow { key: 1, value: 10 },
        KeyedOracleRow { key: 2, value: 20 },
    ];
    let stored_before_snapshot = [
        KeyedOracleRow { key: 1, value: 999 }, // present in both — oracle wins
        KeyedOracleRow { key: 3, value: 30 },  // departed — retained
    ];
    let retained_departed: Vec<KeyedOracleRow> = stored_before_snapshot
        .iter()
        .filter(|stored| !oracle_rows.iter().any(|o| o.key == stored.key))
        .copied()
        .collect();

    let adjusted = keyed_end_state_with_retained_departed_keys(&oracle_rows, &retained_departed);
    assert_eq!(
        adjusted,
        vec![
            KeyedOracleRow { key: 1, value: 10 },
            KeyedOracleRow { key: 2, value: 20 },
            KeyedOracleRow { key: 3, value: 30 },
        ],
        "stored table must equal the oracle's rows plus retained departed keys, exactly \
         once each"
    );
}

/// Plan/classifier-agreement review finding (`docs/plans/
/// 20260809-keyed-frontier.md` Phase 3): `retained_departed_keys_adjusts_
/// the_oracle` (above) already covers an ADDITIVE (`SUM`) keyed recipe over
/// [`KeyedRecipe::new_snapshot_reconcile`]'s `mutable_snapshot`-postured,
/// unclocked driving source — that case refuses via the pre-existing
/// faithful-fold source-posture obligation (`MutableSnapshot` fails
/// obligation 2 regardless of clock), so it never actually exercised the
/// run-shape gate itself. This case swaps the driving source's declared
/// posture to `append_only` (`SourceRecipe::unclocked_append_only_dimension`)
/// while keeping it unclocked — a posture that passes the faithful-fold
/// source-posture obligation on its own — so the ONLY thing that can still
/// refuse a `SUM` fold here is the whole-model run-shape check: this model
/// has no clocked source anywhere (`incremental_shapes.md` §"The two run
/// shapes"), deriving snapshot-reconcile, under which every fold-family
/// column is refused (double-count) regardless of source posture. Before
/// the gate landed, `derive_new_data`'s `Grain::Key` arm admitted a
/// `Technique::KeyedFold` cell here — `smelt explain` showed a cell the
/// runtime classifier (`rules::cumulative::classify_cumulative`,
/// `KeyedSnapshotSourceUnsupportedColumn`) refuses outright.
#[test]
fn snapshot_reconcile_unclocked_append_only_source_with_sum_is_refused() {
    let recipe = KeyedRecipe::new_snapshot_reconcile_unclocked_append_only(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_unclocked_append_only(&recipe, &tmp)
        .expect("stage unclocked append-only keyed recipe");

    let (plan, _diags) = classify_keyed_full(&project, &recipe)
        .expect("classify unclocked append-only keyed recipe");
    let plan = plan.expect(
        "maintenance_plan_report must still return a plan (the universal \
         Backfill cell), even when the targeted keyed fold is refused",
    );

    assert!(
        !plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::NewData { source } if source == &recipe.source.name
        ) || c.technique
            == smelt_logical::maintenance::Technique::KeyedFold),
        "an unclocked (snapshot-reconcile) append-only-postured keyed model must never \
         admit a KeyedFold/NewData cell for a SUM column, regardless of the source's \
         declared MutationProfile: {plan:#?}"
    );
    let refusal_names_snapshot_reconcile_double_count = plan.refusals.iter().any(|r| {
        matches!(
            r,
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, why }
                if trigger.contains(&recipe.source.name)
                    && why.to_lowercase().contains("snapshot-reconcile")
                    && (why.to_lowercase().contains("double-count")
                        || why.to_lowercase().contains("double count"))
        )
    });
    assert!(
        refusal_names_snapshot_reconcile_double_count,
        "expected a NoAdmissibleTechnique refusal naming the snapshot-reconcile \
         double-count reason for source '{}', got: {:#?}",
        recipe.source.name, plan.refusals
    );
}

/// Phase 1 (`docs/plans/20260809-keyed-frontier.md`): the order-monotone
/// overwrite family (`MAX_BY`) grades `Grade::Idempotent`
/// (`crates/smelt-runtime/src/cumulative.rs`'s `WindowedKeyedRule::
/// ledger_grade` doc comment — incumbent-wins re-merge of an
/// already-reflected delta converges) — unlike the additive family
/// (`redelivered_window_refuses_for_additive_keyed`,
/// `crates/smelt-cli/tests/maintenance_conformance/probes.rs`), re-running
/// the SAME window must NOT be refused: no ledger exists for an
/// idempotent-graded cell, and re-merging is harmless by construction.
#[tokio::test]
async fn order_monotone_redelivery_is_idempotent_no_ledger_refusal() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::OrderMonotone);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage order-monotone keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify order-monotone keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the order-monotone keyed recipe to admit at least one cell: {plan:#?}"
    );

    let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_row_keyed(&project, &recipe, &GenRow { d, id: 1, val: 5 }).expect("insert row");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("keyed-order-monotone-1", request.clone())
        .await
        .expect("first fold of the window must succeed");

    let maintained_after_first = {
        let backend = project.backend().await.expect("backend");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot after first fold")
    };

    // Re-deliver the SAME window: an idempotent-graded cell has no ledger
    // and must succeed, converging to the same stored state.
    project
        .run_quiet("keyed-order-monotone-2", request)
        .await
        .expect(
            "re-running an already-folded order-monotone keyed window must succeed — \
             idempotent-graded cells carry no reprocessing ledger",
        );

    let maintained_after_redelivery = {
        let backend = project.backend().await.expect("backend");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot after redelivery")
    };
    assert_eq!(
        maintained_after_first, maintained_after_redelivery,
        "redelivering an already-folded window must converge to byte-identical state, never \
         double-apply the overwrite"
    );
}

/// The once-write family's dedicated constant-payload schedule (shared by
/// [`once_write_pool_upholds_end_state_equivalence`] and phase 8's
/// `once_write_fallback_pool_upholds_end_state_equivalence`/
/// `once_write_multi_candidate_pool_upholds_end_state_equivalence`): the
/// shared key `1` recurs across windows with the SAME value throughout —
/// the once-write provenance proof's own world-fact precondition
/// (`incremental_shapes.md` §"The column-family catalogue") — plus a late
/// redelivery of the already-merged first window.
fn once_write_constant_payload_schedule() -> KeyedSchedule {
    let d1 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");
    KeyedSchedule(vec![
        smelt_maintenance_testkit::recipe::KeyedRunWindow {
            start: d1,
            end: d1 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d1,
                id: 1,
                val: 7,
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
                    val: 7,
                },
                GenRow {
                    d: d2,
                    id: 2,
                    val: 42,
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
                val: 7,
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
/// Written as a targeted (non-generative) case rather than widened into the
/// generated pool: `GenRow::val` is a non-nullable `i64` threaded through
/// the schedule generators, the `STracker` oracle materializer, the feed
/// replay, and the Spark twin's Arrow readers — making it nullable is a
/// generator-wide change out of proportion to the one column family that
/// anticipates NULLs. The oracle here is the same full-refresh body every
/// other keyed case asserts against, evaluated over the physical source
/// table: the schedule is insert-only and every inserted row precedes the
/// run that processes it, so `S` after each run IS the whole source table.
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
    /// `None` stages a NULL payload — the direction `GenRow`'s non-nullable
    /// `val` cannot express.
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
    insert_row_keyed(&project, &recipe, &GenRow { d, id: 1, val: 7 }).expect("insert first row");

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
    insert_row_keyed(&project, &recipe, &GenRow { d, id: 1, val: 99 })
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
async fn once_write_stored_value(
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

// ---------------------------------------------------------------------
// W10 Phase 5 (`docs/plans/20260720-prod-w10-keyed-mutable-admission.md`):
// the change-suppressed column-scoped `MERGE`'s generative conformance leg.
//
// `KeyedRecipe` has no dimension/enrichment support (its model reads exactly
// one source) and `MutableEnrichedRecipe` is `grain: partition` and SELECTS
// its dimension's own attribute column directly — a shape `derive_new_data`
// cannot admit at `grain: key` today (selecting the attribute forces it into
// the fold's column-group, tripping the both-fold-and-enrich refusal Phase 3
// keeps in place). [`KeyedEnrichedRecipe`] is the one reachable shape: a
// `grain: key` fold over an append-only fact source, inner-joined to a
// `mutable_snapshot` dimension declared `allow_full_scan` PURELY for row
// admission — the dimension's own payload column is never selected or
// aggregated, so Phase 2's fold-contribution classifier returns `false` for
// it and Phase 3's waiver admits the source instead of refusing the whole
// plan. This is a fixed pool of one model shape (like `MutableEnrichedRecipe`);
// the generative axis here is the WINDOW SCHEDULE, not the model shape
// (plan Phase 5 "Implementation shape").
// ---------------------------------------------------------------------

/// The fixed fact+dimension `grain: key` shape Phase 4's runtime dispatch
/// reaches: `SELECT <key>, COUNT(<fact>.val) AS event_count FROM
/// smelt.sources.<fact> f JOIN smelt.sources.<dim> dim ON f.<key> = dim.<key>
/// GROUP BY <key>`. Declared inside `gate.rs` rather than added to
/// `smelt-maintenance-testkit` — this phase's Critical files list is
/// `crates/smelt-cli/tests/maintenance_conformance/**` only.
#[derive(Debug, Clone)]
struct KeyedEnrichedRecipe {
    model_name: String,
    fact: SourceRecipe,
    dimension: SourceRecipe,
}

impl KeyedEnrichedRecipe {
    /// The pool's one fixed shape — mirrors [`MutableEnrichedRecipe::new`]'s
    /// own doc comment: exactly one mutable-dimension-enriched keyed shape
    /// needs to be reachable, not a generated construct family.
    fn new() -> Self {
        Self {
            model_name: "recipe_keyed_enriched".to_string(),
            fact: SourceRecipe {
                name: "keyed_enrich_fact".to_string(),
                clock_column: "d".to_string(),
                key_column: "id".to_string(),
                payload_column: "val".to_string(),
                key_shape: KeyShape::Single,
                posture: SourcePosture::AppendOnly,
                key_recurrence: None,
            },
            dimension: SourceRecipe::mutable_dimension("keyed_enrich_dim"),
        }
    }

    /// The model's `SELECT` body: the fact source folded via `COUNT`,
    /// inner-joined to the dimension purely for row admission — the
    /// dimension's own `attr` column is never read, so it never contributes
    /// to the fold (Phase 2's classifier) and stays outside the output's
    /// own column groups.
    fn model_body(&self) -> String {
        let fact_src = format!("smelt.sources.{}", self.fact.name);
        let dim_src = format!("smelt.sources.{}", self.dimension.name);
        let id = &self.fact.key_column;
        let val = &self.fact.payload_column;
        let dim_id = &self.dimension.key_column;
        format!(
            "SELECT f.{id} AS {id}, COUNT(f.{val}) AS event_count FROM {fact_src} f JOIN \
             {dim_src} dim ON f.{id} = dim.{dim_id} GROUP BY f.{id}"
        )
    }

    /// The full model file: `grain: key` frontmatter with a top-level
    /// `unique_key:` (the `RowIdentity::Key` precondition,
    /// `incremental_models.md` §"Per-cell write addressing") and the
    /// dimension declared `allow_full_scan` (its `ColumnScopedMerge` cell's
    /// admission precondition — `incremental_shapes.md` §"Admission matrix"),
    /// mirroring `crates/smelt-runtime/tests/technique_lowering.rs`'s
    /// `keyed_column_scoped_merge_e2e::MODEL_FILE`.
    fn model_file(&self) -> String {
        format!(
            "---\nrefresh: incremental\ngrain: key\nunique_key: {id}\nmaintenance:\n  \
             scan_bounds:\n    per_source:\n      {dim}:\n        allow_full_scan: true\n---\n\
             {body}\n",
            id = self.fact.key_column,
            dim = self.dimension.name,
            body = self.model_body(),
        )
    }

    /// The oracle query for this recipe: [`Self::model_body`] with the fact
    /// source reference swapped for `fact_table_ref` (a full-refresh oracle
    /// or an `STracker`-materialized `S_k` temp table) and the dimension's
    /// reference swapped for its physical table — mirrors
    /// [`MutableEnrichedRecipe::oracle_body_over`], except this recipe never
    /// mutates its dimension, so "current physical state" and "state at
    /// staging time" always coincide.
    fn oracle_body_over(&self, fact_table_ref: &str) -> String {
        self.model_body()
            .replace(&format!("smelt.sources.{}", self.fact.name), fact_table_ref)
            .replace(
                &format!("smelt.sources.{}", self.dimension.name),
                &format!("main.sources_{}", self.dimension.name),
            )
    }
}

/// Ids seeded into the staged dimension table, wide enough to cover every id
/// [`arb_keyed_schedule`] can generate (2-3 windows, up to 2 fresh ids per
/// window on top of the one shared re-touched key) plus this test's own
/// hand-appended zero-change redelivery window.
const KEYED_ENRICHED_DIM_SEED_MAX_ID: i64 = 150;

/// Stage a [`KeyedEnrichedRecipe`] into a fresh temp project + DuckDB file —
/// the keyed-enriched-pool counterpart of [`stage_mixed_recipe`]/
/// [`stage_keyed_recipe`]: writes both source YAMLs + the model file,
/// creates both physical source tables, and pre-seeds the dimension with one
/// row per id in `1..=KEYED_ENRICHED_DIM_SEED_MAX_ID` (`attr = id * 100`) so
/// every fact row a generated schedule inserts already has a matching
/// dimension row to join against.
fn stage_keyed_enriched_recipe(
    recipe: &KeyedEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact.source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension.source_yaml(),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    for id in 1..=KEYED_ENRICHED_DIM_SEED_MAX_ID {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one row into a [`KeyedEnrichedRecipe`]'s staged fact source table.
fn insert_fact_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.fact.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Insert one row into a [`KeyedEnrichedRecipe`]'s staged dimension source
/// table — the "add a dim row matching existing facts" genuine-membership-
/// change window (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
fn insert_dim_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES ({id}, {attr})",
            recipe.dimension.name,
        ),
        [],
    )?;
    Ok(())
}

/// Update a [`KeyedEnrichedRecipe`]'s staged dimension row's `attr` column —
/// the "change a joined attribute" window. The recipe's own model body never
/// selects `attr` (module doc comment on [`KeyedEnrichedRecipe::model_body`]),
/// so this mutation is deliberately invisible in the maintained output; it
/// exercises the membership-recompute dispatch firing and reproducing the
/// oracle without corruption, not a value change.
fn update_dim_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{} SET {} = {attr} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.payload_column, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Delete a [`KeyedEnrichedRecipe`]'s staged dimension row — the genuine-
/// departure window: a fact row keyed on `id` may already be admitted
/// (joined to this dim row), so removing it must make that fact disappear
/// from the maintained output entirely, not merely go stale.
fn delete_dim_row_keyed_enriched(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`KeyedEnrichedRecipe`] through the real maintenance
/// derivation — the keyed-enriched-pool counterpart of
/// [`classify_keyed_full`]/[`classify_mixed`]. Unlike the resolver-level
/// proof in [`keyed_enriched_recipe_admits_membership_recompute`]
/// (which calls `resolve_live_membership_recompute_cell` directly and never
/// consults the model's OTHER triggers), this goes through
/// `smelt_db::maintenance_plan_report`/`file_diagnostics` — the SAME
/// multi-trigger derivation `derive_model_maintenance_plan_impl` runs for
/// every trigger the model has (including the `NewData` trigger Phase 3's
/// waiver governs) — so a regression in the waiver surfaces here even
/// though it would NOT surface in the resolver-only proof (the resolver
/// only ever looks up the `UpstreamMutation` cell by trigger, independent
/// of whether a sibling `NewData` trigger was refused).
fn classify_keyed_enriched_full(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged keyed-enriched-pool model {:?} (expected at {}) not found among discovered \
             models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// `keyed_enriched_recipe_admits_membership_recompute` (rewrite of
/// `keyed_enriched_recipe_admits_suppressed_column_scoped_merge`,
/// `docs/plans/20260808-membership-sensitivity.md` Phase 3): the recipe's
/// dimension is read purely in the `JOIN`'s `ON` predicate — a row-admission
/// read — so per `incremental_models.md` §"The plan matrix" its derived plan
/// now carries a membership-sensitive `UpstreamMutation(dim)` cell assigned
/// `Technique::DeleteInsert` (the recompute family), never
/// `Technique::ColumnScopedMerge` (Phase 1's review checklist: "membership
/// cells cannot receive `ColumnScopedMerge`"), WITHOUT any diagnostic
/// refusing the model overall, AND for which
/// `resolve_live_membership_recompute_cell` — the exact resolver
/// `execute.rs`'s `plan_is_keyed` branch calls alongside
/// `resolve_live_column_scoped_cell` — resolves
/// `WriteSuppression::Suppressed`
/// (`crates/smelt-runtime/tests/technique_lowering.rs`'s
/// `keyed_membership_recompute_e2e::resolves_suppressed_membership_recompute_for_keyed_dimension_cell`
/// unit-level proof, generalized to this pool's own recipe). Guards against
/// silent degradation back to `Unconditional`-only, outright refusal of the
/// `UpstreamMutation` cell, or the whole model dying at `execute_project`'s
/// pre-execution diagnostic gate with `MaintenanceNoAdmissibleTechnique`
/// even though the `UpstreamMutation` cell itself resolves fine in
/// isolation.
#[test]
fn keyed_enriched_recipe_admits_membership_recompute() {
    let recipe = KeyedEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_enriched_recipe(&recipe, &tmp).expect("stage keyed-enriched recipe");

    let (plan, diagnostics) =
        classify_keyed_enriched_full(&project, &recipe).expect("classify keyed-enriched recipe");
    let plan = plan.expect("maintenance_plan_report must return a plan for the staged recipe");

    let dim_source = recipe.dimension.name.clone();
    assert!(
        plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::UpstreamMutation { source } if source == &dim_source
        ) && c.technique == Technique::DeleteInsert),
        "expected an UpstreamMutation({dim_source}) cell with Technique::DeleteInsert (the \
         membership-sensitive recompute family) in the derived plan, got: {plan:#?}"
    );
    assert!(
        !plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::UpstreamMutation { source } if source == &dim_source
        ) && c.technique == Technique::ColumnScopedMerge),
        "a membership-sensitive cell must never receive Technique::ColumnScopedMerge — it \
         cannot fix which rows exist, only rewrite already-admitted rows' columns"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.severity == smelt_db::DiagnosticSeverity::Error),
        "the staged keyed-enriched recipe must produce zero Error diagnostics: {diagnostics:#?}"
    );

    let text = recipe.model_file();
    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("single-model file");
    };
    let sql_body = &text[sql_offset..];

    let sources = vec![
        SourceFacts {
            name: recipe.fact.name.clone(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: recipe.dimension.name.clone(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        },
    ];
    let mut explicitly_mutable = std::collections::HashSet::new();
    explicitly_mutable.insert(recipe.dimension.name.clone());

    let (source, cell, suppression) = resolve_live_membership_recompute_cell(
        sql_body,
        &recipe.model_name,
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
    )
    .expect("resolver must not error")
    .expect(
        "a live DeleteInsert membership-recompute cell must resolve for the enrich-only \
         mutable dimension — if this fails, admission has regressed back to refusing the \
         whole plan or to only an Unconditional write (choice::resolve_write_variant)",
    );

    assert_eq!(source, recipe.dimension.name);
    assert_eq!(cell.technique, Technique::DeleteInsert);
    assert!(
        matches!(suppression, WriteSuppression::Suppressed { .. }),
        "expected the change-suppressed matched arm, got {suppression:?}"
    );
}

/// Default deterministic case count for
/// `keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery` —
/// small, since every case drives several real `execute_project` windows
/// plus one appended redelivery window.
const KEYED_ENRICHED_DEFAULT_CASES: usize = 4;

fn keyed_enriched_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_KEYED_ENRICHED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(KEYED_ENRICHED_DEFAULT_CASES)
}

/// The end-state equivalence assertion for a [`KeyedEnrichedRecipe`] — the
/// keyed-enriched-pool counterpart of [`assert_keyed_equivalence`]. Unlike
/// [`assert_mixed_settled`]'s `OracleMode` gating (needed because a
/// `grain: partition` model's column-scoped merge only ever settles its
/// window on the NEXT catch-up run), the membership-recompute technique
/// this recipe now dispatches through recomputes the model's FULL current
/// state every run (`resolve_live_membership_recompute_cell`'s own
/// `candidate_select`), so equivalence holds after every window
/// unconditionally, even once the dimension itself starts being mutated
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
async fn assert_keyed_enriched_equivalence(
    project: &LinkCProject,
    recipe: &KeyedEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "keyed-enriched end-state equivalence violated for model {:?} at run {k}: \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// The fixed dimension key every generated [`KeyedEnrichedRecipe`] schedule
/// case's hand-built dim-mutation windows exercise, chosen well outside both
/// [`KEYED_ENRICHED_DIM_SEED_MAX_ID`]'s pre-seeded range and
/// `arb_keyed_schedule`'s own generated id space (`KEYED_SHARED_KEY_ID = 1`
/// plus a `next_id` counter starting at 100, incrementing by at most 6 per
/// case) — so it never collides with a generated fact row's own id.
const DIM_MUTATION_TEST_ID: i64 = 9001;

/// `keyed_enriched_pool_upholds_equivalence_under_dim_mutation` (rewrite of
/// `keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery`,
/// `docs/plans/20260808-membership-sensitivity.md` Phase 3): drives a
/// generated [`KeyedSchedule`] against [`KeyedEnrichedRecipe`] through the
/// real `execute_project` pipeline (`stage_keyed_enriched_recipe` +
/// `LinkCProject::run_quiet`), asserting end-state equivalence against the
/// full-refresh oracle after every window, THEN appends four hand-built
/// windows that genuinely mutate the dimension — the point of this rewrite,
/// since the generated schedule alone never un-admits or newly admits a
/// fact (the dimension is pre-seeded wide enough to already cover every
/// generated id):
///
/// 1. a fresh fact row keyed on [`DIM_MUTATION_TEST_ID`], with no matching
///    dim row yet — must stay un-admitted, same as the full-refresh oracle's
///    own inner join.
/// 2. a dim row added matching that now-unmatched fact — a genuine new
///    admission only the recompute family (never `ColumnScopedMerge`,
///    which cannot create rows) can pick up.
/// 3. the dim row's `attr` mutated — invisible in the output (the recipe
///    never selects `attr`), proving the dispatch fires and reproduces the
///    oracle without corruption on a mutation that changes nothing
///    observable.
/// 4. the dim row deleted — a genuine departure: `DIM_MUTATION_TEST_ID` DOES
///    have a currently-admitted fact, so this is exactly the scenario
///    `emit_staged_candidate_conditional`'s (pre-Phase-3) `DELETE` left
///    stale — the row must now disappear from the maintained output.
///
/// Finally, one hand-built zero-change window (no new fact rows, no
/// dimension mutation) closes the change-suppressed
/// `WriteSuppression::Suppressed` arm's no-op path — including proving the
/// NEW departed-key `DELETE` predicate (window 4 above) is itself a no-op
/// once nothing has departed since: the maintained table's full contents
/// are asserted byte-identical before and after.
#[test]
fn keyed_enriched_pool_upholds_equivalence_under_dim_mutation() {
    let n = keyed_enriched_case_count();
    let mut runner = TestRunner::deterministic();
    let schedule_strat = arb_keyed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedEnrichedRecipe::new();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_enriched_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: keyed-enriched recipe failed to stage: {e}"));

        let mut tracker = STracker::new(&recipe.fact);
        let mut last_window_end: Option<chrono::NaiveDate> = None;

        rt.block_on(async {
            for (w, window) in schedule.0.iter().enumerate() {
                for row in &window.rows {
                    insert_fact_row_keyed_enriched(&project, &recipe, row)
                        .unwrap_or_else(|e| panic!("case {i}: insert fact row failed: {e}"));
                }

                let snapshot = {
                    let conn = project.connect().expect("connect");
                    read_source_snapshot(&conn, &recipe.fact)
                };

                let mut request = base_request("dev");
                request.start = Some(window.start.format("%Y-%m-%d").to_string());
                request.end = Some(window.end.format("%Y-%m-%d").to_string());
                let outcome = project
                    .run_quiet(&format!("keyed-enriched-run-{i}-{w}"), request)
                    .await
                    .unwrap_or_else(|e| panic!("case {i}: window {w} run failed: {e}"));

                let record = outcome
                    .models
                    .get(&recipe.model_name)
                    .unwrap_or_else(|| panic!("case {i}: model did not run in window {w}"));
                if w == 0 {
                    assert_ne!(
                        record.strategy, "delete_insert_suppressed",
                        "case {i}: the creation run must not take the membership-recompute \
                         path — the target doesn't exist yet"
                    );
                } else {
                    assert_eq!(
                        record.strategy, "delete_insert_suppressed",
                        "case {i}: window {w} must dispatch the keyed run loop through the \
                         staged-candidate membership-recompute technique once the target \
                         exists"
                    );
                }

                let k = tracker.record_run(window.start, window.end, snapshot);
                assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                    .await
                    .unwrap_or_else(|e| {
                        panic!("case {i}: window {w} equivalence check failed: {e}")
                    });
                last_window_end = Some(window.end);
            }

            let mut next_start = last_window_end.expect("schedule generated at least one window");
            let mut run_dim_mutation_window = |label: &'static str| {
                let start = next_start;
                let end = start + chrono::Duration::days(1);
                next_start = end;
                (label, start, end)
            };

            // Window 1: a fresh fact row with no matching dim row yet — must
            // stay un-admitted.
            let (label, start, end) = run_dim_mutation_window("unmatched-fact");
            insert_fact_row_keyed_enriched(
                &project,
                &recipe,
                &GenRow {
                    d: start,
                    id: DIM_MUTATION_TEST_ID,
                    val: 42,
                },
            )
            .unwrap_or_else(|e| panic!("case {i}: {label}: insert fact row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));
            {
                let conn = project.connect().expect("connect");
                let admitted: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM main.{} WHERE id = {DIM_MUTATION_TEST_ID}",
                            recipe.model_name
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .expect("count admitted rows");
                assert_eq!(
                    admitted, 0,
                    "case {i}: {label}: a fact with no matching dim row must not be admitted"
                );
            }

            // Window 2: add the matching dim row — a genuine new admission.
            let (label, start, end) = run_dim_mutation_window("dim-add-admits");
            insert_dim_row_keyed_enriched(&project, &recipe, DIM_MUTATION_TEST_ID, 900_100)
                .unwrap_or_else(|e| panic!("case {i}: {label}: insert dim row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: {label}: model did not run"));
            assert_eq!(record.strategy, "delete_insert_suppressed");
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));
            {
                let conn = project.connect().expect("connect");
                let event_count: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT event_count FROM main.{} WHERE id = {DIM_MUTATION_TEST_ID}",
                            recipe.model_name
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .expect("newly admitted row must exist");
                assert_eq!(
                    event_count, 1,
                    "case {i}: {label}: the newly admitted fact must be folded correctly"
                );
            }

            // Window 3: change the dim row's `attr` — never selected by the
            // model body, so invisible in the output; only proves the
            // dispatch fires without corruption.
            let (label, start, end) = run_dim_mutation_window("dim-attr-change-invisible");
            update_dim_row_keyed_enriched(&project, &recipe, DIM_MUTATION_TEST_ID, 900_199)
                .unwrap_or_else(|e| panic!("case {i}: {label}: update dim row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: {label}: model did not run"));
            assert_eq!(record.strategy, "delete_insert_suppressed");
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));

            // Window 4: delete the dim row — a genuine departure.
            // `DIM_MUTATION_TEST_ID` DOES have a currently-admitted fact, so
            // this is exactly the scenario the pre-Phase-3 region-scoped
            // emitter left stale.
            let (label, start, end) = run_dim_mutation_window("dim-delete-departs");
            delete_dim_row_keyed_enriched(&project, &recipe, DIM_MUTATION_TEST_ID)
                .unwrap_or_else(|e| panic!("case {i}: {label}: delete dim row failed: {e}"));
            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(start.format("%Y-%m-%d").to_string());
            request.end = Some(end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: {label}: model did not run"));
            assert_eq!(record.strategy, "delete_insert_suppressed");
            let k = tracker.record_run(start, end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: equivalence check failed: {e}"));
            {
                let conn = project.connect().expect("connect");
                let survives: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM main.{} WHERE id = {DIM_MUTATION_TEST_ID}",
                            recipe.model_name
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .expect("count surviving rows");
                assert_eq!(
                    survives, 0,
                    "case {i}: {label}: a genuinely departed dim row's fact must be DELETED \
                     from the maintained output, not left stale"
                );
            }

            // Zero-change redelivery: a fresh, never-processed window with
            // no new fact rows and no dimension mutation. The live
            // `UpstreamMutation` cell still dispatches (known divergence —
            // unconditional per-run dispatch, `incremental_models.md`
            // §Known Divergences), but nothing has changed since window 4,
            // so this exercises the change-suppressed arm's genuine no-op
            // path — INCLUDING the new departed-key DELETE predicate, which
            // must itself be a no-op now that nothing has departed since
            // the last run.
            let (label, redelivery_start, redelivery_end) = run_dim_mutation_window("redelivery");

            let maintained_before = {
                let backend = project.backend().await.expect("backend");
                snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                    .await
                    .expect("snapshot before redelivery")
            };

            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.fact)
            };
            let mut request = base_request("dev");
            request.start = Some(redelivery_start.format("%Y-%m-%d").to_string());
            request.end = Some(redelivery_end.format("%Y-%m-%d").to_string());
            let outcome = project
                .run_quiet(&format!("keyed-enriched-run-{i}-{label}"), request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: {label}: run failed: {e}"));
            let record = outcome
                .models
                .get(&recipe.model_name)
                .unwrap_or_else(|| panic!("case {i}: model did not run on redelivery"));
            assert_eq!(
                record.strategy, "delete_insert_suppressed",
                "case {i}: the zero-change redelivery window must still dispatch the \
                 staged-candidate membership-recompute technique"
            );

            let k = tracker.record_run(redelivery_start, redelivery_end, snapshot);
            assert_keyed_enriched_equivalence(&project, &recipe, &tracker, k)
                .await
                .unwrap_or_else(|e| panic!("case {i}: redelivery equivalence check failed: {e}"));

            let maintained_after = {
                let backend = project.backend().await.expect("backend");
                snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                    .await
                    .expect("snapshot after redelivery")
            };
            assert_eq!(
                maintained_before, maintained_after,
                "case {i}: the change-suppressed arm (and its departed-key DELETE predicate) \
                 must write nothing observable when nothing changed — the maintained table's \
                 contents must be byte-identical before and after the zero-change redelivery \
                 run"
            );
        });
    }
}

/// Snapshot `main.<table>`'s full contents as sorted, comparable text rows —
/// the zero-write redelivery step's before/after equality check.
pub(crate) async fn snapshot_table_rows(
    backend: &dyn Backend,
    table: &str,
) -> anyhow::Result<Vec<Vec<String>>> {
    let batches = backend
        .execute_sql(&format!("SELECT * FROM main.{table} ORDER BY ALL"))
        .await?;
    let mut rows = Vec::new();
    for batch in &batches {
        for row_idx in 0..batch.num_rows() {
            let mut row = Vec::new();
            for col in batch.columns() {
                row.push(arrow::util::display::array_value_to_string(col, row_idx)?);
            }
            rows.push(row);
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// `docs/plans/20260809-sensitivity-precision.md` Phase 5: the
// closure-pruned column-scoped `MERGE` pool (`ValueEnrichedRecipe`).
// ---------------------------------------------------------------------

/// Ids seeded into the staged dimension table, wide enough to cover the
/// fixed set of ids this fixed-shape test drives by hand (mirrors
/// [`KEYED_ENRICHED_DIM_SEED_MAX_ID`]'s convention, scaled down since this
/// test drives a fixed schedule rather than a generated one).
const VALUE_ENRICHED_DIM_SEED_MAX_ID: i64 = 20;

/// Stage a [`ValueEnrichedRecipe`] into a fresh temp project + DuckDB file —
/// the closure-pruned-enrichment-pool counterpart of
/// [`stage_keyed_enriched_recipe`]: writes both source YAMLs + the model
/// file, creates both physical source tables, and pre-seeds the dimension
/// with one row per id in `1..=VALUE_ENRICHED_DIM_SEED_MAX_ID`
/// (`attr = id * 100`).
fn stage_value_enriched_recipe(
    recipe: &ValueEnrichedRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension_source_yaml(),
    )?;
    // `smelt.yml`, NOT the SQL frontmatter, carries this recipe's
    // `models.<name>.merge_key:` — the top-level replacement for the retired
    // `batched.unique_key` sub-block (`docs/specs/models.md` §"Batched
    // sub-block retirement"), and the only surface for
    // `PartitionGrainConfig.unique_key` under `grain: partition`
    // (`ValueEnrichedRecipe::model_file`'s own doc comment explains why the
    // SQL-frontmatter form can't carry it — `merge_key:` never confers
    // identity). This is the column-scoped `MERGE`'s own `ON`-predicate key
    // (`decide_column_merge_dispatch`'s `model_declares_unique_key`
    // precondition) — without it the live `ColumnScopedMerge` cell resolves
    // in the derived plan but never actually dispatches at execution time.
    let smelt_yml = format!(
        "{base}models:\n  {model}:\n    merge_key: [{id}]\n",
        base = render::render_smelt_yml(&db_path),
        model = recipe.model_name,
        id = recipe.fact.key_column,
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml)?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    for id in 1..=VALUE_ENRICHED_DIM_SEED_MAX_ID {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir, db_path)
}

/// Insert one row into a [`ValueEnrichedRecipe`]'s staged fact source table.
fn insert_fact_row_value_enriched(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.fact.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Update a [`ValueEnrichedRecipe`]'s staged dimension row's `attr` column —
/// the value-mutation window this recipe's whole point is: the model DOES
/// select `attr` directly, so this must become visible in the maintained
/// output through the column-scoped `MERGE`, never a recompute fallback.
fn update_dim_row_value_enriched(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    id: i64,
    attr: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "UPDATE main.sources_{} SET {} = {attr} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.payload_column, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Delete a [`ValueEnrichedRecipe`]'s staged dimension row — the
/// departed-dimension-row window: since the join is `LEFT JOIN` and closed
/// (never membership-sensitive for this shape), the fact row must SURVIVE
/// with `attr` re-derived to NULL, never disappear from the output.
fn delete_dim_row_value_enriched(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    id: i64,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "DELETE FROM main.sources_{} WHERE {} = {id}",
            recipe.dimension.name, recipe.dimension.key_column,
        ),
        [],
    )?;
    Ok(())
}

/// Classify a staged [`ValueEnrichedRecipe`] through the real maintenance
/// derivation — the closure-pruned-enrichment-pool counterpart of
/// [`classify_keyed_enriched_full`], going through
/// `smelt_db::maintenance_plan_report`/`file_diagnostics` (the SAME
/// Salsa-backed derivation the LSP/CLI diagnostics use), not a hand-built
/// `ModelInputs` (unlike
/// `smelt-logical/tests/maintenance_tracer.rs::closed_outer_enrichment_join_upstream_mutation_derives_column_scoped_merge`,
/// which this test's plan-shape assertion mirrors end-to-end).
fn classify_value_enriched_full(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged value-enriched-pool model {:?} (expected at {}) not found among discovered \
             models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// The end-state equivalence assertion for a [`ValueEnrichedRecipe`] — the
/// closure-pruned-enrichment-pool counterpart of
/// [`assert_keyed_enriched_equivalence`]. The column-scoped `MERGE` this
/// recipe dispatches through recomputes every existing key's `attr` column
/// every run (the accepted-full-scan corner), so equivalence holds
/// unconditionally after every window, exactly like the membership-recompute
/// counterpart.
async fn assert_value_enriched_equivalence(
    project: &LinkCProject,
    recipe: &ValueEnrichedRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = recipe.oracle_body_over(&format!("oracle_{}", recipe.fact.name));
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "value-enriched end-state equivalence violated for model {:?} at run {k}: \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// The fixed dimension key this test's hand-built windows exercise —
/// pre-seeded (`VALUE_ENRICHED_DIM_SEED_MAX_ID`) so its initial fact row
/// admits with a real, non-NULL `attr` before any mutation.
const VALUE_ENRICHED_TEST_ID: i64 = 7;

/// `value_enriched_recipe_executes_column_scoped_merge`
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 5): the derived
/// plan for a closure-pruned `LEFT JOIN` enrichment carries
/// `Technique::ColumnScopedMerge`/`Corner::ColumnMerge` for its dimension's
/// `UpstreamMutation` cell (never `Technique::DeleteInsert` — the
/// membership-recompute family a still-open closure would fall back to),
/// and driving the recipe through the real `execute_project` pipeline
/// against a real DuckDB actually DISPATCHES that technique
/// (`RunOutcome.models[..].strategy == "column_scoped_merge"`, the same
/// observable `keyed_enriched_pool_upholds_equivalence_under_dim_mutation`
/// uses to distinguish `delete_insert_suppressed` from a silent recompute
/// fallback) across a dimension VALUE mutation, a dimension ROW DELETION
/// (the departed-dimension-row case: the fact row survives with `attr`
/// re-derived to NULL, since the join never drops rows), and a zero-change
/// redelivery — matching the independently-staged full-refresh oracle at
/// every step.
#[test]
fn value_enriched_recipe_executes_column_scoped_merge() {
    let recipe = ValueEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_value_enriched_recipe(&recipe, &tmp).expect("stage value-enriched recipe");

    // --- (a) Plan-shape assertion: ColumnScopedMerge, never DeleteInsert.
    let (plan, diagnostics) =
        classify_value_enriched_full(&project, &recipe).expect("classify value-enriched recipe");
    let plan = plan.unwrap_or_else(|| {
        panic!(
            "maintenance_plan_report must return a plan for the staged recipe; diagnostics: \
             {diagnostics:#?}"
        )
    });
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.severity == smelt_db::DiagnosticSeverity::Error),
        "the staged value-enriched recipe must produce zero Error diagnostics: {diagnostics:#?}"
    );

    let dim_source = recipe.dimension.name.clone();
    let attr_cell = plan
        .cells
        .iter()
        .find(|c| {
            matches!(&c.trigger, Trigger::UpstreamMutation { source } if source == &dim_source)
                && c.group == "{attr}"
        })
        .unwrap_or_else(|| {
            panic!("no {{attr}} UpstreamMutation({dim_source}) cell in derived plan: {plan:#?}")
        });
    assert_eq!(
        attr_cell.technique,
        Technique::ColumnScopedMerge,
        "the closure-pruned LEFT JOIN's own ON read must not make {{attr}} membership-\
         sensitive — expected ColumnScopedMerge, got {:?} (plan: {plan:#?})",
        attr_cell.technique
    );
    assert_eq!(attr_cell.corner, Corner::ColumnMerge);
    assert!(
        !plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::UpstreamMutation { source } if source == &dim_source
        ) && c.technique == Technique::DeleteInsert),
        "a closure-pruned enrichment must never fall back to the membership-recompute family: \
         {plan:#?}"
    );

    // --- (b)-(d): drive the real pipeline and assert dispatch + equivalence.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut tracker = STracker::new(&recipe.fact);

    rt.block_on(async {
        // Creation run: seeds the fact row this test mutates around.
        let start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
        let end = start + chrono::Duration::days(1);
        insert_fact_row_value_enriched(
            &project,
            &recipe,
            &GenRow {
                d: start,
                id: VALUE_ENRICHED_TEST_ID,
                val: 42,
            },
        )
        .expect("insert seed fact row");
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet("value-enriched-creation", request)
            .await
            .expect("creation run");
        let record = outcome
            .models
            .get(&recipe.model_name)
            .expect("model ran on creation");
        assert_ne!(
            record.strategy, "column_scoped_merge",
            "the creation run must not take the column-scoped MERGE path — the target doesn't \
             exist yet"
        );
        let k = tracker.record_run(start, end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .expect("creation-run equivalence");

        // Every subsequent window RE-TOUCHES the creation run's own
        // `[start, end)` window rather than advancing forward: the
        // column-scoped `MERGE`'s write stays scoped to exactly the run's
        // own batch window (`used_column_scoped_merge`'s doc comment in
        // `execute.rs`: "keeps the write scoped to exactly the window a
        // DELETE+INSERT would have touched"), so a mutation is only visible
        // once a run actually re-touches the window the mutated row's fact
        // lives in — mirroring a real catch-up run over an already-processed
        // partition, not a forward advance into fresh territory.
        let run_window = |label: &'static str| (label, start, end);

        // (b) Dimension VALUE mutation: `attr` changes for an already-
        // admitted row — must become visible via a real column-scoped
        // MERGE, matching the oracle.
        let (label, start, end) = run_window("dim-value-mutation");
        update_dim_row_value_enriched(&project, &recipe, VALUE_ENRICHED_TEST_ID, 900_700)
            .unwrap_or_else(|e| panic!("{label}: update dim row failed: {e}"));
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet(&format!("value-enriched-run-{label}"), request)
            .await
            .unwrap_or_else(|e| panic!("{label}: run failed: {e}"));
        let record = outcome
            .models
            .get(&recipe.model_name)
            .unwrap_or_else(|| panic!("{label}: model did not run"));
        assert_eq!(
            record.strategy, "column_scoped_merge",
            "{label}: expected the live column-scoped MERGE to dispatch, got {:?}",
            record.strategy
        );
        let k = tracker.record_run(start, end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .unwrap_or_else(|e| panic!("{label}: equivalence check failed: {e}"));
        {
            let conn = project.connect().expect("connect");
            let attr: i64 = conn
                .query_row(
                    &format!(
                        "SELECT attr FROM main.{} WHERE id = {VALUE_ENRICHED_TEST_ID}",
                        recipe.model_name
                    ),
                    [],
                    |row| row.get(0),
                )
                .expect("mutated row must exist with the new attr value");
            assert_eq!(
                attr, 900_700,
                "{label}: the value mutation must be visible through the column-scoped MERGE"
            );
        }

        // (c) Dimension ROW DELETION: a genuine departure from the dim, but
        // NOT from the maintained output (LEFT JOIN) — the fact row must
        // survive with `attr` re-derived to NULL.
        let (label, start, end) = run_window("dim-row-deletion");
        delete_dim_row_value_enriched(&project, &recipe, VALUE_ENRICHED_TEST_ID)
            .unwrap_or_else(|e| panic!("{label}: delete dim row failed: {e}"));
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(start.format("%Y-%m-%d").to_string());
        request.end = Some(end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet(&format!("value-enriched-run-{label}"), request)
            .await
            .unwrap_or_else(|e| panic!("{label}: run failed: {e}"));
        let record = outcome
            .models
            .get(&recipe.model_name)
            .unwrap_or_else(|| panic!("{label}: model did not run"));
        assert_eq!(
            record.strategy, "column_scoped_merge",
            "{label}: expected the live column-scoped MERGE to dispatch, got {:?}",
            record.strategy
        );
        let k = tracker.record_run(start, end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .unwrap_or_else(|e| panic!("{label}: equivalence check failed: {e}"));
        {
            let conn = project.connect().expect("connect");
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT val, attr FROM main.{} WHERE id = {VALUE_ENRICHED_TEST_ID}",
                    recipe.model_name
                ))
                .expect("prepare survivor read-back");
            let mut rows = stmt.query([]).expect("query survivor row");
            let row = rows
                .next()
                .expect("query row")
                .expect("{label}: the departed-dimension fact row must survive, not disappear");
            let val: i64 = row.get(0).expect("val");
            let attr: Option<i64> = row.get(1).expect("attr");
            assert_eq!(val, 42, "{label}: the fact's own column must be unchanged");
            assert_eq!(
                attr, None,
                "{label}: attr must re-derive to NULL once the dim row departs, since the LEFT \
                 JOIN never drops the fact row"
            );
        }

        // (d) Zero-change redelivery: idempotent — a re-run of an
        // already-caught-up window must write nothing observable.
        let (label, redelivery_start, redelivery_end) = run_window("redelivery");
        let maintained_before = {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                .await
                .expect("snapshot before redelivery")
        };
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.fact)
        };
        let mut request = base_request("dev");
        request.start = Some(redelivery_start.format("%Y-%m-%d").to_string());
        request.end = Some(redelivery_end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet(&format!("value-enriched-run-{label}"), request)
            .await
            .unwrap_or_else(|e| panic!("{label}: run failed: {e}"));
        let record = outcome
            .models
            .get(&recipe.model_name)
            .unwrap_or_else(|| panic!("{label}: model did not run"));
        assert_eq!(
            record.strategy, "column_scoped_merge",
            "{label}: expected the live column-scoped MERGE to dispatch, got {:?}",
            record.strategy
        );
        let k = tracker.record_run(redelivery_start, redelivery_end, snapshot);
        assert_value_enriched_equivalence(&project, &recipe, &tracker, k)
            .await
            .unwrap_or_else(|e| panic!("{label}: redelivery equivalence check failed: {e}"));

        let maintained_after = {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                .await
                .expect("snapshot after redelivery")
        };
        assert_eq!(
            maintained_before, maintained_after,
            "{label}: the redelivery run (idempotent re-merge) must write nothing observable \
             when nothing changed — the maintained table's contents must be byte-identical \
             before and after"
        );
    });
}

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
                val: 10,
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

// ---------------------------------------------------------------------
// Phase 8: the `SimulatedChangeFeed` step family — recompute-only
// admission for `change_feed`-declared sources
// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 8;
// `incremental_models.md` §Known Divergences' `change_feed`-scoping entry).
// ---------------------------------------------------------------------

/// Default deterministic case count for `change_feed_source_admits_recompute_only`.
const FEED_ADMISSION_DEFAULT_CASES: usize = 10;

fn feed_admission_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_FEED_ADMISSION_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FEED_ADMISSION_DEFAULT_CASES)
}

/// `change_feed_source_admits_recompute_only` (plan Phase 8 TDD list): a
/// `change_feed`-declared source's admitted cells are all full-input
/// re-derivation, never a fold (`incremental_models.md` §Known Divergences:
/// "no live fold machinery consumes a change feed's delta shape yet" —
/// mirrors `crates/smelt-logical/tests/maintenance_coverage_matrix.rs`'s
/// `ex14_change_feed_sum_recompute_only`/`ex26_change_feed_latest_writer_recompute_only`,
/// but driven through the real production entry point
/// (`smelt_db::maintenance_plan_report`) rather than the pure derivation
/// directly).
#[test]
fn change_feed_source_admits_recompute_only() {
    let n = feed_admission_case_count();
    let mut runner = TestRunner::deterministic();
    let combiner_strat = arb_keyed_combiner();

    let mut checked = 0;
    for i in 0..n {
        let combiner = combiner_strat.new_tree(&mut runner).unwrap().current();
        let recipe = feed::feed_keyed_recipe(combiner);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = feed::stage_feed_keyed(&recipe, &project_dir, &db_path)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage feed-driven keyed recipe: {e}"));

        let (plan, diags) = classify_keyed_full(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify failed: {e}"));
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no maintenance plan returned at all: diagnostics={diags:#?}")
        });

        assert!(
            !plan.cells.is_empty(),
            "case {i}: change_feed-driven keyed recipe {recipe:?} admitted zero cells \
             (expected at least the universal Backfill recompute cell): diagnostics={diags:#?}"
        );
        for cell in &plan.cells {
            assert_eq!(
                cell.technique,
                Technique::DeleteInsert,
                "case {i}: a change_feed-declared source must admit ONLY full-input \
                 re-derivation (Technique::DeleteInsert), never a fold — got {:?} for cell \
                 {cell:?}",
                cell.technique,
            );
        }
        assert!(
            !plan.cells.iter().any(|c| matches!(
                &c.trigger,
                Trigger::NewData { source } if source == &recipe.source.name
            )),
            "case {i}: a change_feed source must never admit a targeted NewData fold cell \
             today (incremental_models.md §Known Divergences' change_feed-scoping entry): \
             {plan:#?}"
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "N={n} deterministic sample never staged a change_feed-driven keyed recipe — \
         generator/derivation regression"
    );
}

/// Default deterministic case count for
/// `feed_declared_source_upholds_equivalence_via_recompute`.
const FEED_RECOMPUTE_DEFAULT_CASES: usize = 6;

fn feed_recompute_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_FEED_RECOMPUTE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FEED_RECOMPUTE_DEFAULT_CASES)
}

/// Generous upper bound on distinct dimension ids this test's fact rows
/// ever reference — mirrors `MIXED_DIM_SEED_MAX_ID` (Phase 4's own
/// convention above).
const FEED_DIM_SEED_MAX_ID: i64 = 12;

/// Wrap `sql` restricted to the single partition day `day` on column
/// `day_col` — used to isolate one window's own rows so a genuinely
/// incremental run's freshness (or an older, un-revisited window's frozen
/// staleness) can be checked without the whole-table noise of every other
/// window.
fn restrict_to_day(sql: &str, day: chrono::NaiveDate, day_col: &str) -> String {
    format!(
        "SELECT * FROM ({sql}) t WHERE t.{day_col} = DATE '{}'",
        day.format("%Y-%m-%d")
    )
}

/// `feed_declared_source_upholds_equivalence_via_recompute` (plan Phase 8
/// TDD list): mutation schedules over feed-declared sources settle to
/// full-refresh equality. Drives the fact+`change_feed`-dimension mixed
/// shape (`feed::stage_feed_enriched`) rather than the `grain: key` pool:
/// `change_feed_source_admits_recompute_only` already pins that a
/// `grain: key` model with a fold spec over a `change_feed` source carries a
/// build-blocking `MaintenanceNoAdmissibleTechnique` Error diagnostic (fold
/// refused), so it can never actually be driven through `execute_project`
/// — only the classify-level admission surface is checkable there. The
/// mixed shape builds cleanly (no `UpstreamMutation` cell is EVER
/// constructed for a `change_feed`-declared dimension — `incremental_models.md`
/// §Known Divergences' `change_feed`-scoping entry — so there is nothing to
/// refuse).
///
/// Unlike `mutable_pool_settles_to_full_refresh`'s sibling pattern, there is
/// no `UpstreamMutation` cell to make an already-materialized window catch
/// up: this test drives GENUINE incremental (`full_refresh: false`) runs —
/// one fresh partition per schedule step, interleaved with a dimension
/// mutation applied just before it — and checks two things a full-refresh-
/// only drive (the prior, weaker version of this test) could never catch:
/// (1) freshness — a NEWLY computed window always reflects the dimension's
/// CURRENT state (`maintenance.scan_bounds...allow_full_scan: true` means
/// the join is never scan-bounded), so a regression that fed a stale/cached
/// dimension snapshot into a fresh incremental compute would fail here; (2)
/// the documented staleness itself — the FIRST window, once materialized,
/// is provably never revisited by any later incremental run (the
/// `change_feed`-scoping divergence), so it diverges from a live recompute
/// after the schedule's mutations land, exactly the `incremental_models.md`
/// §Known Divergences contract. A final `full_refresh: true` run must then
/// settle the WHOLE table back to equivalence — that is the "via recompute"
/// half of this test's name, now actually exercised after a real
/// incremental history rather than skipped entirely.
#[test]
fn feed_declared_source_upholds_equivalence_via_recompute() {
    let n = feed_recompute_case_count();
    let mut runner = TestRunner::deterministic();
    let schedule_strat = feed::arb_feed_step_schedule(FeedSourcePosture::ChangeFeed);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let day_col = MutableEnrichedRecipe::new().fact.clock_column.clone();

    for i in 0..n {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = MutableEnrichedRecipe::new();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project =
            feed::stage_feed_enriched(&recipe, &project_dir, &db_path, FEED_DIM_SEED_MAX_ID)
                .unwrap_or_else(|e| {
                    panic!("case {i}: failed to stage feed-enriched mixed recipe: {e}")
                });

        let day0 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");

        // One fact row per pre-seeded dimension id, so the join always
        // produces output regardless of which dimension rows the schedule
        // below goes on to mutate/retract.
        for id in 1..=FEED_DIM_SEED_MAX_ID {
            insert_fact_row(
                &project,
                &recipe,
                &GenRow {
                    d: day0,
                    id,
                    val: id * 10,
                },
            )
            .unwrap_or_else(|e| panic!("case {i}: failed to seed fact row {id}: {e}"));
        }

        rt.block_on(async {
            let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
            let live_oracle_sql =
                recipe.oracle_body_over(&format!("main.sources_{}", recipe.fact.name));

            // Genuinely incremental first run: materialize day0 only.
            let mut day0_request = base_request("dev");
            day0_request.start = Some(day0.format("%Y-%m-%d").to_string());
            day0_request.end = Some(
                (day0 + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string(),
            );
            project
                .run_quiet(&format!("feed-run-{i}-day0"), day0_request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: initial incremental day0 run failed: {e}"));

            {
                let backend = project
                    .backend()
                    .await
                    .expect("backend for day0 freshness check");
                let maintained_day0 = restrict_to_day(&maintained_sql, day0, &day_col);
                let oracle_day0 = restrict_to_day(&live_oracle_sql, day0, &day_col);
                assert!(
                    multiset_equal_via_backend(backend.as_ref(), &maintained_day0, &oracle_day0)
                        .await
                        .expect("day0 freshness multiset comparison"),
                    "case {i}: freshly incremental-computed day0 must match a live recompute \
                     over the dimension's state at computation time: maintained \
                     ({maintained_day0:?}) != oracle ({oracle_day0:?})"
                );
            }

            // Snapshot day0 the moment it settles — this pool's own frozen
            // reference for the staleness check below.
            let day0_snapshot_sql = {
                let conn = project.connect().expect("connect for day0 snapshot");
                let snapshot = restrict_to_day(&maintained_sql, day0, &day_col);
                // Materialize into a real (non-TEMP) table, since later
                // read-backs open fresh connections and a TEMP table is
                // scoped to the connection that created it — so later runs
                // (which mutate `main.sources_<dim>`, not the model table
                // itself) can't change what this reference query returns.
                conn.execute_batch(&format!(
                    "CREATE TABLE main.feed_day0_snapshot_{i} AS {snapshot}"
                ))
                .unwrap_or_else(|e| panic!("case {i}: failed to snapshot day0: {e}"));
                format!("SELECT * FROM main.feed_day0_snapshot_{i}")
            };

            for (step_i, step) in schedule.0.iter().enumerate() {
                {
                    let conn = project.connect().expect("connect for feed step");
                    feed::apply_feed_step(&conn, &recipe.dimension, step, step_i as i64)
                        .unwrap_or_else(|e| {
                            panic!("case {i} step {step_i}: apply_feed_step failed: {e}")
                        });
                }

                // A genuinely NEW window, never touched before: one fresh
                // fact row for a stable pre-seeded id, on a day this
                // schedule has not run before. Its incremental computation
                // happens strictly AFTER the mutation just applied above, so
                // — per `allow_full_scan: true` — it must reflect the
                // dimension's post-mutation state.
                let new_day = day0 + chrono::Duration::days(step_i as i64 + 1);
                let dim_id = (step_i as i64 % FEED_DIM_SEED_MAX_ID) + 1;
                insert_fact_row(
                    &project,
                    &recipe,
                    &GenRow {
                        d: new_day,
                        id: dim_id,
                        val: dim_id * 10 + step_i as i64,
                    },
                )
                .unwrap_or_else(|e| {
                    panic!("case {i} step {step_i}: failed to insert new-window fact row: {e}")
                });

                let mut request = base_request("dev");
                request.start = Some(new_day.format("%Y-%m-%d").to_string());
                request.end = Some(
                    (new_day + chrono::Duration::days(1))
                        .format("%Y-%m-%d")
                        .to_string(),
                );
                project
                    .run_quiet(&format!("feed-run-{i}-{step_i}"), request)
                    .await
                    .unwrap_or_else(|e| panic!("case {i} step {step_i}: run failed: {e}"));

                let backend = project.backend().await.expect("backend for read-back");

                // Freshness: the window just computed must match a live
                // recompute over the CURRENT (post-mutation) dimension
                // state — proves this is a real incremental run, not a
                // no-op, and that it isn't silently reading a stale
                // dimension snapshot.
                let maintained_new_day = restrict_to_day(&maintained_sql, new_day, &day_col);
                let oracle_new_day = restrict_to_day(&live_oracle_sql, new_day, &day_col);
                assert!(
                    multiset_equal_via_backend(
                        backend.as_ref(),
                        &maintained_new_day,
                        &oracle_new_day
                    )
                    .await
                    .expect("new-window multiset comparison"),
                    "case {i} step {step_i}: freshly incremental-computed window {new_day} must \
                     match a live recompute over the dimension's current state: maintained \
                     ({maintained_new_day:?}) != oracle ({oracle_new_day:?}), schedule={schedule:?}"
                );
            }

            // Documented current behavior (`incremental_models.md` §Known
            // Divergences, `change_feed`-scoping entry): no incremental run
            // ever revisits day0 once materialized, so it stays frozen at
            // its original computation-time snapshot even though the
            // schedule above has since mutated the dimension rows day0
            // joined against.
            {
                let backend = project
                    .backend()
                    .await
                    .expect("backend for staleness check");
                let maintained_day0_now = restrict_to_day(&maintained_sql, day0, &day_col);
                assert!(
                    multiset_equal_via_backend(
                        backend.as_ref(),
                        &maintained_day0_now,
                        &day0_snapshot_sql
                    )
                    .await
                    .expect("frozen-day0 multiset comparison"),
                    "case {i}: day0 must remain frozen at its original computation-time state \
                     across purely incremental runs (no UpstreamMutation cell is ever built for \
                     a change_feed-declared dimension) — maintained day0 changed without a \
                     revisiting run: now ({maintained_day0_now:?}) != snapshot \
                     ({day0_snapshot_sql:?}), schedule={schedule:?}"
                );

                // And the flip side: that frozen day0 means the WHOLE table
                // is now stale relative to a live recompute — this is the
                // exact risk this test guards against (a silent regression
                // that either (a) fixed this staleness without settling via
                // a documented path, or (b) broke fresh-window correctness
                // and happened to still "settle" by accident).
                assert!(
                    !multiset_equal_via_backend(
                        backend.as_ref(),
                        &maintained_sql,
                        &live_oracle_sql
                    )
                    .await
                    .expect("whole-table staleness multiset comparison"),
                    "case {i}: expected the WHOLE table to be stale relative to a live recompute \
                     after purely incremental runs following dimension mutations — if this now \
                     holds, either the change_feed-scoping divergence has been fixed (update \
                     this test's doc comment and drop this assertion) or the schedule failed to \
                     mutate anything day0 actually joined against, schedule={schedule:?}"
                );
            }

            // Full-refresh recompute must still settle the WHOLE table back
            // to equivalence — the "via recompute" contract this test is
            // named for, now exercised after a real incremental history.
            let mut refresh_request = base_request("dev");
            refresh_request.full_refresh = true;
            project
                .run_quiet(&format!("feed-run-{i}-full-refresh"), refresh_request)
                .await
                .unwrap_or_else(|e| panic!("case {i}: final full-refresh run failed: {e}"));

            let backend = project
                .backend()
                .await
                .expect("backend for final read-back");
            assert!(
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &live_oracle_sql)
                    .await
                    .expect("final full-refresh multiset comparison"),
                "case {i}: feed-declared source equivalence via full-refresh recompute violated \
                 after an incremental history: maintained ({maintained_sql:?}) != oracle \
                 ({live_oracle_sql:?}), schedule={schedule:?}"
            );
        });
    }
}

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
                val: 30,
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
fn deployed_column_names(project: &LinkCProject, table: &str) -> Vec<String> {
    let file_store = smelt_state::file_store::FileStore::new(
        &project.project_dir,
        "dev",
        smelt_core::config::StateMode::Environments,
    );
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
fn derive_plan_with_real_deployed_schema(
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
        smelt_logical::maintenance::availability::StateAvailability::all(),
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
        rt_insert_and_run(&project, &recipe, w1_start, w1_end, &[GenRow { d: w1_start, id: 1, val: 10 }], &mut tracker)
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
        insert_row(&project, &recipe, &GenRow { d: w2_start, id: 2, val: 20 })
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
async fn rt_insert_and_run(
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
/// snapshot must carry `Refusal::SkeletonColumnAdded`, never a
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
            val: 10,
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
            smelt_logical::maintenance::Refusal::SkeletonColumnAdded { column }
                if column == &recipe.source.key_column
        )),
        "a GROUP BY widening add must refuse SkeletonColumnAdded naming {:?}: {plan:#?}",
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

/// `skeleton_position_add_is_refused_or_recomputed_never_corrupted` (plan
/// Phase 9 TDD list): adding a column in a grouping/skeleton position
/// mid-schedule never yields a silently-wrong maintained state — either a
/// named refusal (via the real [`classify`] verdict protocol) or a full
/// recompute whose result equals the rewritten body's own oracle. The
/// recompute is driven via `FullRefreshRun`, the only technique that
/// actually survives a schema-shape change today — see
/// `column_add_between_runs_recovers_equivalence`'s doc comment for why a
/// plain windowed re-run against an already-materialized table hits a raw
/// column-count mismatch instead.
#[test]
fn skeleton_position_add_is_refused_or_recomputed_never_corrupted() {
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

    let mut tracker = STracker::new(&recipe.source);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    // Pre-rewrite run, under the original body — establishes a green
    // baseline before the skeleton-changing rewrite.
    rt.block_on(insert_row(
        &project,
        &recipe,
        &GenRow {
            d: w1_start,
            id: 1,
            val: 10,
        },
    ))
    .expect("seed row");
    let snapshot = {
        let conn = project.connect().expect("connect");
        read_source_snapshot(&conn, &recipe.source)
    };
    let mut request = base_request("dev");
    request.start = Some(w1_start.format("%Y-%m-%d").to_string());
    request.end = Some(w1_end.format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-0", request))
        .expect("initial run before rewrite");
    let k0 = tracker.record_run(w1_start, w1_end, snapshot);
    rt.block_on(assert_equivalence(&project, &recipe, &tracker, k0))
        .expect("pre-rewrite equivalence");

    // Rewrite: add the source's row-key column into the GROUP BY — a
    // grain/skeleton change, `incremental_models.md`'s `SkeletonAdd`
    // territory.
    std::fs::write(
        project
            .project_dir
            .join(format!("models/{}.sql", recipe.model_name)),
        render::render_model_file_with_edit(&recipe, ModelEdit::AddGroupingColumn),
    )
    .expect("write rewritten model file");

    // Classify the NOW-rewritten project through the real derivation
    // (`verdict.rs`'s fail-loud protocol: a refusal always carries a named
    // Maintenance*/admission diagnostic, never an unexplained empty plan).
    let post_verdict = classify(&project, &recipe).expect("classify after rewrite");
    match post_verdict {
        Verdict::Refused(diags) => {
            assert!(
                !diags.is_empty(),
                "a refused skeleton-position rewrite must carry a named diagnostic \
                 (verdict.rs's own fail-loud check already enforces this — asserted here too \
                 so this test would fail if that changed)"
            );
        }
        Verdict::Admitted(_) => {
            let mut request2 = base_request("dev");
            request2.full_refresh = true;
            rt.block_on(project.run_quiet("run-1", request2))
                .expect("an admitted post-rewrite plan must not fail to execute");

            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.source)
            };
            let k1 = tracker.record_full_refresh(snapshot);
            rt.block_on(assert_equivalence_with_edit(
                &project,
                &recipe,
                &tracker,
                k1,
                Some(ModelEdit::AddGroupingColumn),
            ))
            .expect(
                "an admitted skeleton-position rewrite's recompute must equal the rewritten \
                 body's own oracle — never silently diverge",
            );
        }
    }
}

// ---------------------------------------------------------------------
// Phase A6: the composed (`grain: key` + `timeseries:`) recipe family,
// covering all three key-temporal-locality routes
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A6;
// `incremental_shapes.md` §"Key temporal locality").
//
// Route 1 (key-embedded) is driven through the real `execute_project`
// pipeline, exactly like the keyed pool above. Routes 2 (key-determined)
// and 3 (recurrence-bounded, declared) are admitted by the real
// `establish_locality` gate over real staged frontmatter/YAML
// (`classify_composed_full`), but drive their actual merge mechanics
// through `run_windowed_keyed_maintenance` directly against a real
// `DuckDbBackend` — the documented, pre-existing workaround
// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs` already
// uses (see `ComposedKeyedRecipe`'s own doc comment for why).
// ---------------------------------------------------------------------

/// Default deterministic case count for `composed_keyed_pool_upholds_equivalence`.
const COMPOSED_DEFAULT_CASES: usize = 6;

fn composed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_COMPOSED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(COMPOSED_DEFAULT_CASES)
}

/// Classify a staged [`ComposedKeyedRecipe`] through the real maintenance
/// derivation — the composed-pool counterpart of `classify_keyed_full`.
/// Returns the derived plan (possibly with zero cells / a locality refusal)
/// plus every diagnostic on the target model.
pub fn classify_composed_full(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged composed-pool model {:?} (expected at {}) not found among discovered models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// Assert `recipe`'s plan clears the locality gate with the expected
/// [`LocalitySlice`] shape for its own [`ComposedRoute`] — the single
/// per-case admission check every drive path below relies on having
/// already passed.
fn assert_composed_admitted_with_expected_route(
    recipe: &ComposedKeyedRecipe,
    plan: &smelt_logical::maintenance::MaintenancePlan,
) -> anyhow::Result<()> {
    if plan.refusals.iter().any(|r| {
        matches!(
            r,
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
        )
    }) {
        anyhow::bail!(
            "composed recipe {:?} (route {:?}) was refused by the locality gate: {:?}",
            recipe.model_name,
            recipe.route,
            plan.refusals
        );
    }
    let Some(key_locality) = plan.key_locality.as_ref() else {
        anyhow::bail!(
            "composed recipe {:?} (route {:?}) admitted a plan with no key_locality",
            recipe.model_name,
            recipe.route
        );
    };
    match (recipe.route, &key_locality.slice) {
        (ComposedRoute::KeyEmbedded, LocalitySlice::Window { .. }) => Ok(()),
        (ComposedRoute::KeyDetermined, LocalitySlice::DeltaValues { .. }) => Ok(()),
        (ComposedRoute::RecurrenceBounded, LocalitySlice::RecurrenceBounded { .. }) => Ok(()),
        (route, slice) => {
            anyhow::bail!(
                "composed recipe {:?}: route {:?} admitted an unexpected slice shape: {:?}",
                recipe.model_name,
                route,
                slice
            )
        }
    }
}

// ---- Route 1 (key-embedded): full `execute_project` drive -----------

fn insert_composed_row(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Whole-table equivalence for route 1: the maintained output equals the
/// model body evaluated over the full, currently-inserted source table —
/// route 1's schedule never reprocesses a window, so no `STracker`
/// S-restriction is needed.
async fn assert_composed_route1_equivalence(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = render::render_composed_oracle_sql(recipe);
    if !multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await? {
        anyhow::bail!(
            "composed route-1 equivalence violated for {:?}: maintained ({maintained_sql:?}) != \
             oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Per-slice equivalence for route 1 (`incremental_models.md` §"Per-slice
/// equivalence"): the stored rows of one output slice (`d = slice_date`)
/// equal the model SQL evaluated over the source rows within that slice's
/// derived reach — zero margin here (`SIMPLE_SQL`-shaped, no lookback), so
/// the reach is exactly the source rows sharing that same date.
async fn assert_composed_route1_per_slice(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
    slice_date: chrono::NaiveDate,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    let d = slice_date.format("%Y-%m-%d");
    let maintained_sql = format!(
        "SELECT * FROM main.{} WHERE d = DATE '{d}'",
        recipe.model_name
    );
    let oracle_body = render::render_composed_oracle_sql(recipe);
    let oracle_sql = format!("SELECT * FROM ({oracle_body}) t WHERE d = DATE '{d}'");
    if !multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await? {
        anyhow::bail!(
            "composed route-1 per-slice equivalence violated for {:?} at slice {d}: maintained \
             ({maintained_sql:?}) != model SQL over the slice's derived reach ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

async fn drive_composed_route1_and_assert(
    project: &LinkCProject,
    recipe: &ComposedKeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    for (i, window) in schedule.0.iter().enumerate() {
        for row in &window.rows {
            insert_composed_row(project, recipe, row)?;
        }

        let mut request = base_request("dev");
        request.start = Some(window.start.format("%Y-%m-%d").to_string());
        request.end = Some(window.end.format("%Y-%m-%d").to_string());
        project
            .run_quiet(&format!("composed-route1-run-{i}"), request)
            .await?;

        assert_composed_route1_equivalence(project, recipe).await?;
        assert_composed_route1_per_slice(project, recipe, window.start).await?;
    }
    Ok(())
}

// ---- Routes 2/3: direct-driver execution against a real DuckDbBackend --

/// The driving source's own `timeseries:` declaration every composed
/// recipe's classification carries (`event_time_column`/`partition_column`
/// both `d`, `day` granularity — the fixed `events(d, id, val)` shape).
fn composed_driving_timeseries() -> smelt_core::config::TimeseriesConfig {
    smelt_core::config::TimeseriesConfig {
        event_time_column: "d".to_string(),
        partition_column: "d".to_string(),
        granularity: smelt_core::config::Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn composed_route2_classification(recipe: &ComposedKeyedRecipe) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "total".to_string(),
            per_partition_agg: "SUM".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(composed_driving_timeseries()),
        },
    }
}

fn composed_route3_classification(recipe: &ComposedKeyedRecipe) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: format!("smelt.sources.{}", recipe.source.name),
            timeseries: Some(composed_driving_timeseries()),
        },
    }
}

fn composed_route3_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    }
}

/// One window's own row list, rendered as a literal `VALUES` relation —
/// the per-step delta is built directly from the window's own rows rather
/// than filtered off a physical table by a `d = <date>` predicate, which
/// would wrongly require a redelivered row's own event-time to equal the
/// window that delivers it (`ComposedRoute3Window`'s own doc comment names
/// this as exactly the "out-of-order redelivery" shape the pool must be
/// able to express).
fn composed_delta_values_sql(rows: &[GenRow]) -> String {
    let values: Vec<String> = rows
        .iter()
        .map(|r| format!("({}, DATE '{}', {})", r.id, r.d.format("%Y-%m-%d"), r.val))
        .collect();
    format!("(VALUES {}) AS t(id, d, val)", values.join(", "))
}

fn composed_route2_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, CAST(d AS DATE) AS pdate, SUM(val) AS total FROM {} GROUP BY id, d",
        composed_delta_values_sql(rows)
    )
}

fn composed_route3_delta_sql(rows: &[GenRow]) -> String {
    format!(
        "SELECT id, MAX(d) AS last_seen FROM {} GROUP BY id",
        composed_delta_values_sql(rows)
    )
}

/// The route-2 oracle: `pdate` is write-once (never re-merged — see
/// `ComposedKeyedRecipe`'s doc comment), so its true end-state value is the
/// event-time of whichever window *first* delivered that key — the
/// minimum `d` across all of that key's accumulated rows (every row in
/// this pool's route-2 schedule carries `d == its own window's run date`,
/// and windows always run in ascending order).
fn composed_route2_oracle_sql(source_name: &str) -> String {
    format!(
        "SELECT id, CAST(MIN(d) AS DATE) AS pdate, SUM(val) AS total FROM main.sources_{source_name} \
         GROUP BY id"
    )
}

fn composed_route3_oracle_sql(source_name: &str) -> String {
    format!("SELECT id, MAX(d) AS last_seen FROM main.sources_{source_name} GROUP BY id")
}

/// Convert a batch of Arrow results into a sorted `Vec` of `(column,
/// value)` row vectors — a multiset comparator over two such `Vec`s (via
/// plain `==` after sorting) that does not require a `duckdb::Connection`
/// (`oracle::multiset_equal`'s own contract), since routes 2/3 query
/// through a live `DuckDbBackend` instead (mirrors
/// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`'s own
/// `execute_sql`-only discipline — never open a second, independent
/// connection to the same DuckDB file while the backend holds one open).
fn rows_as_sorted_multiset(batches: &[arrow::array::RecordBatch]) -> Vec<Vec<(String, String)>> {
    let mut rows: Vec<Vec<(String, String)>> = batches_to_rows(batches)
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();
    rows.sort();
    rows
}

async fn assert_backend_multiset_equal(
    backend: &DuckDbBackend,
    left_sql: &str,
    right_sql: &str,
    context: &str,
) -> anyhow::Result<()> {
    let left = backend.execute_sql(left_sql).await?;
    let right = backend.execute_sql(right_sql).await?;
    let left_rows = rows_as_sorted_multiset(&left);
    let right_rows = rows_as_sorted_multiset(&right);
    if left_rows != right_rows {
        anyhow::bail!(
            "{context}: multiset mismatch\n  left  ({left_sql:?}): {left_rows:?}\n  right \
             ({right_sql:?}): {right_rows:?}"
        );
    }
    Ok(())
}

async fn assert_composed_route2_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_route2_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-2 equivalence",
    )
    .await
}

/// Per-slice equivalence for route 2 (`incremental_models.md` §"Per-slice
/// equivalence"): route 2 never settles by date — its slice is the
/// delta's own partition **values**, not a date-range window — so the
/// natural slice here is one distinct `pdate` value; each such slice must
/// equal the oracle restricted to that same value.
async fn assert_composed_route2_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(pdate AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE pdate = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE pdate = DATE '{v}'",
            composed_route2_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-2 per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

async fn assert_composed_route3_equivalence(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = composed_route3_oracle_sql(&recipe.source.name);
    assert_backend_multiset_equal(
        backend,
        &maintained_sql,
        &oracle_sql,
        "composed route-3 equivalence",
    )
    .await
}

/// Per-slice equivalence for route 3: `last_seen` genuinely settles
/// (`AfterRecurrenceBound`), so each distinct `last_seen` date-value slice
/// must equal the oracle restricted to that same value.
async fn assert_composed_route3_per_slice(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
) -> anyhow::Result<()> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT DISTINCT CAST(last_seen AS VARCHAR) AS v FROM main.{}",
            recipe.model_name
        ))
        .await?;
    let values: Vec<String> = batches_to_rows(&batches)
        .into_iter()
        .filter_map(|r| r.get("v").cloned())
        .collect();
    for v in values {
        let maintained_sql = format!(
            "SELECT * FROM main.{} WHERE last_seen = DATE '{v}'",
            recipe.model_name
        );
        let oracle_sql = format!(
            "SELECT * FROM ({}) t WHERE last_seen = DATE '{v}'",
            composed_route3_oracle_sql(&recipe.source.name)
        );
        assert_backend_multiset_equal(
            backend,
            &maintained_sql,
            &oracle_sql,
            "composed route-3 per-slice equivalence",
        )
        .await?;
    }
    Ok(())
}

/// Append `rows` to the driving source's accumulation-log table
/// (`main.sources_<name>`, created by `render::stage_composed`) — used only
/// as the oracle's own read side for routes 2/3; the direct driver's own
/// per-step delta never reads this table (`composed_delta_values_sql`'s doc
/// comment).
async fn insert_composed_rows_via_backend(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    rows: &[GenRow],
) -> anyhow::Result<()> {
    for row in rows {
        backend
            .execute_sql(&format!(
                "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                row.d.format("%Y-%m-%d"),
                row.id,
                row.val,
            ))
            .await?;
    }
    Ok(())
}

/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase C6
/// TDD item 4: the composed pool runs with suppression enabled and must
/// stay equivalent under redelivery schedules — `total`/`last_seen` are
/// both registry-backed deterministic aggregates (`SUM`/`MAX`), Comparable
/// under the P3 change-comparability walk, over the recipe's own proven
/// `id` key, so a hand-built `Suppressed` verdict here mirrors exactly what
/// `resolve_write_suppression` would resolve for these fixed classifications
/// (`crate::cumulative::resolve_cumulative_write_suppression`'s own
/// production wiring), without re-deriving the walk over generated SQL this
/// testkit's classifications are never actually parsed from.
fn composed_route2_suppression() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["total".to_string()],
    }
}

fn composed_route3_suppression() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["last_seen".to_string()],
    }
}

async fn drive_composed_route2_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &KeyedSchedule,
) -> anyhow::Result<()> {
    let classification = composed_route2_classification(recipe);
    // `Some(&composed_route2_slice())` — the real `DeltaValues` slice a
    // route-2 model is admitted with — is deliberately **not** passed
    // here. Doing so renders `emit_keyed_fold`'s `target.<col> IN (SELECT
    // DISTINCT <col> FROM (<delta_select>))` predicate, and real DuckDB
    // (confirmed directly against the `duckdb` CLI, v1.5.4/v1.10504)
    // refuses to bind ANY `MERGE` whose `ON` clause combines a derived
    // `USING` subquery with that `IN (SELECT DISTINCT … FROM (subquery))`
    // shape at all — `Invalid Input Error: BindMerge - expected to find an
    // operator of type LOGICAL_GET but got FILTER` — independently of
    // whether the delta is a `VALUES` literal or a real table scan. This
    // is a genuine DuckDB backend limitation for the `DeltaValues`
    // slice-predicate shape, recorded verbatim in `incremental_models.md`
    // §Known Divergences under "Key temporal locality" (the paragraph
    // starting "Route 2's slice-pruned merge … is unexercised against a
    // real backend") and cross-referenced again in that section's §Tests
    // bullet — distinct from the already-documented NOT-NULL nullability
    // blocker. Fixing the emitted
    // predicate shape is production code in `smelt-logical::maintenance::
    // emit`, outside this testkit-only phase's Critical files — flagged
    // here rather than silently worked around. Passing `None` still
    // exercises the real merge mechanics this test actually asserts
    // (write-once `pdate`, additive `total`) against real DuckDB; only the
    // target-scan **pruning** optimisation itself goes unexercised
    // (`incremental_shapes.md` §"Key temporal locality": "pruning is not a
    // write clamp" — every delta row still merges with or without it).
    let slice: Option<&LocalitySlice> = None;

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route2_delta_sql(&rows))
        };
        let steps = driving_steps(
            &window.start.format("%Y-%m-%d").to_string(),
            &window.end.format("%Y-%m-%d").to_string(),
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            slice,
            &composed_route2_suppression(),
            compile_step,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("composed route-2 window {i} merge failed: {e}"))?;

        assert_composed_route2_equivalence(backend, recipe).await?;
        assert_composed_route2_per_slice(backend, recipe).await?;
    }
    Ok(())
}

async fn drive_composed_route3_and_assert(
    backend: &DuckDbBackend,
    recipe: &ComposedKeyedRecipe,
    schedule: &ComposedRoute3Schedule,
) -> anyhow::Result<()> {
    let classification = composed_route3_classification(recipe);
    let slice = composed_route3_slice();

    for (i, window) in schedule.0.iter().enumerate() {
        insert_composed_rows_via_backend(backend, recipe, &window.rows).await?;

        let rows = window.rows.clone();
        let compile_step = move |_step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
            Ok(composed_route3_delta_sql(&rows))
        };
        let run_date_str = window.run_date.format("%Y-%m-%d").to_string();
        let next_day_str = (window.run_date + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let steps = driving_steps(
            &run_date_str,
            &next_day_str,
            &smelt_core::config::Granularity::Day,
        )?;
        run_windowed_keyed_maintenance(
            backend,
            &recipe.model_name,
            "main",
            &recipe.model_name,
            &steps,
            &classification,
            Some(&slice),
            &composed_route3_suppression(),
            compile_step,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "composed route-3 window {i} (in-bound redelivery) unexpectedly refused: {e}"
            )
        })?;

        assert_composed_route3_equivalence(backend, recipe).await?;
        assert_composed_route3_per_slice(backend, recipe).await?;
    }
    Ok(())
}

/// `composed_keyed_pool_upholds_equivalence` (plan Phase A6 TDD list): the
/// standing proptest gate over the composed pool — deterministic seed,
/// small N. Each case draws one of the three [`ComposedRoute`]s, stages
/// it, confirms the real locality gate admits it with the expected slice
/// shape, then drives its equivalence check (route 1 through real
/// `execute_project`; routes 2/3 through the direct driver against a real
/// `DuckDbBackend`) — asserting equivalence, and a per-slice probe, after
/// **every** step.
#[test]
fn composed_keyed_pool_upholds_equivalence() {
    let n = composed_case_count();
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();
    let keyed_schedule_strat = arb_keyed_schedule();
    let route3_schedule_strat = arb_composed_route3_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = render::stage_composed(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });

        let (plan, diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no plan derived for {recipe:?}: diagnostics={diags:#?}")
        });
        assert_composed_admitted_with_expected_route(&recipe, &plan).unwrap_or_else(|e| {
            panic!("case {i}: {e}");
        });
        admitted_cases += 1;

        match route {
            ComposedRoute::KeyEmbedded => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                rt.block_on(drive_composed_route1_and_assert(
                    &project, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-1 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
            ComposedRoute::KeyDetermined => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_route2_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-2 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
            ComposedRoute::RecurrenceBounded => {
                let schedule = route3_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_route3_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-3 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic composed sample admitted zero cases — generator/derivation \
         regression"
    );
}

/// `composed_keyed_admission_rate_stays_above_floor` (plan Phase A6 TDD
/// list): generator health — every generated composed recipe is
/// deliberately constructed to admit exactly its own route (unlike the
/// append-only-partition pool's randomly-drawn constructs, this pool has
/// no randomised refusal branch), so the floor is high: a regression that
/// silently breaks one route's admission must fail this test rather than
/// hollow out the standing gate above unnoticed.
#[test]
fn composed_keyed_admission_rate_stays_above_floor() {
    const N: usize = 30;
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();

    let mut admitted = 0;
    for i in 0..N {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = render::stage_composed(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });
        let (plan, _diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let admitted_here = plan
            .map(|p| assert_composed_admitted_with_expected_route(&recipe, &p).is_ok())
            .unwrap_or(false);
        if admitted_here {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.90,
        "composed-pool admission rate {rate:.2} over N={N} fell below the 90% floor \
         ({admitted}/{N} admitted) — a route-admission regression would silently hollow out the \
         standing gate"
    );
}

// =============================================================================
// Phase C5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
// — T1 (change-suppressed keyed-fold MERGE) vs T2 (staged-candidate
// conditional DELETE+INSERT) vs the full-refresh oracle, over the keyed
// pool's own shape (a `unique_key`-addressed region), at a fixed processed-
// input set `S`. The three techniques must be interchangeable
// (`docs/specs/model_transforms.md` §"Change-suppressed MERGE and the
// staged-candidate conditional DELETE+INSERT" — "the fixed-`S` bit-equality
// obligation"): given the identical seed state and the identical candidate
// delta, all three end states agree.
// =============================================================================

/// Seed three independently-named tables (T1's MERGE target, T2's staged-
/// candidate target, and the full-refresh oracle) with identical state, then
/// drive each to the same fixed `S` via its own technique, asserting all
/// three end states agree as multisets. `run_marker` proves suppression
/// actually happened (not merely that the bits match): a row whose fold
/// result reproduces the stored value keeps its prior marker under both T1
/// and T2, while a changed or brand-new row picks up the new run's marker.
#[tokio::test]
async fn keyed_pool_t1_t2_and_full_refresh_agree_at_fixed_s() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("db.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    for table in ["t1_target", "t2_target", "oracle"] {
        backend
            .execute_sql(&format!(
                "CREATE TABLE main.{table} (device_id BIGINT, event_count BIGINT, run_marker \
                 VARCHAR)"
            ))
            .await
            .expect("create table");
        backend
            .execute_sql(&format!(
                "INSERT INTO main.{table} VALUES (1, 5, 'run1'), (2, 3, 'run1'), (3, 8, 'run1')"
            ))
            .await
            .expect("seed table");
    }

    // Fixed `S`: device 1 gets no new events (unchanged-effect re-run);
    // device 2's delta genuinely changes the combined result; device 3 is
    // absent from this run's delta entirely (out of the touched region);
    // device 4 is brand new.
    let delta_values = "(1, 0, 'run2'), (2, 4, 'run2'), (4, 6, 'run2')";
    let key = vec!["device_id".to_string()];
    let compared_columns = vec!["event_count".to_string()];

    // T1: change-suppressed keyed-fold MERGE.
    let folds = vec![
        (
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        ),
        ("run_marker".to_string(), "delta.run_marker".to_string()),
    ];
    let t1_group = smelt_logical::maintenance::emit::emit_keyed_fold_suppressed(
        "main.t1_target",
        &key,
        &folds,
        &format!("SELECT * FROM (VALUES {delta_values}) AS t(device_id, event_count, run_marker)"),
        None,
        &compared_columns,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&t1_group)
        .await
        .expect("T1 change-suppressed keyed-fold merge must succeed");

    // T2: staged-candidate conditional DELETE+INSERT. Its candidate select
    // must carry the fully-combined row (the same effect the MERGE's fold
    // expression computes), since T2 has no combiner of its own — it
    // re-derives full candidate rows and diffs them against stored state.
    let t2_candidate_select = "SELECT t.device_id, t.event_count + d.delta_count AS event_count, \
                                d.new_marker AS run_marker FROM main.t2_target t JOIN (SELECT * \
                                FROM (VALUES (1, 0, 'run2'), (2, 4, 'run2')) AS \
                                x(device_id, delta_count, new_marker)) AS d ON t.device_id = \
                                d.device_id UNION ALL SELECT 4, 6, 'run2'";
    let t2_group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.t2_target",
        "__smelt_staged_t2_target",
        &key,
        t2_candidate_select,
        &compared_columns,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&t2_group)
        .await
        .expect("T2 staged-candidate conditional write must succeed");

    // Full-refresh oracle: recompute the whole region directly.
    backend
        .execute_sql(
            "UPDATE main.oracle SET event_count = 5, run_marker = 'run1' WHERE device_id = 1",
        )
        .await
        .expect("oracle: device 1 unchanged");
    backend
        .execute_sql(
            "UPDATE main.oracle SET event_count = 7, run_marker = 'run2' WHERE device_id = 2",
        )
        .await
        .expect("oracle: device 2 changed");
    backend
        .execute_sql("INSERT INTO main.oracle VALUES (4, 6, 'run2')")
        .await
        .expect("oracle: device 4 new");

    // All three end states are multiset-equal over the addressed columns.
    assert_backend_multiset_equal(
        &backend,
        "SELECT device_id, event_count FROM main.t1_target",
        "SELECT device_id, event_count FROM main.oracle",
        "T1 (change-suppressed keyed-fold MERGE) vs full-refresh oracle",
    )
    .await
    .expect("T1 must equal the full-refresh oracle at fixed S");
    assert_backend_multiset_equal(
        &backend,
        "SELECT device_id, event_count FROM main.t2_target",
        "SELECT device_id, event_count FROM main.oracle",
        "T2 (staged-candidate conditional DELETE+INSERT) vs full-refresh oracle",
    )
    .await
    .expect("T2 must equal the full-refresh oracle at fixed S");
    assert_backend_multiset_equal(
        &backend,
        "SELECT device_id, event_count FROM main.t1_target",
        "SELECT device_id, event_count FROM main.t2_target",
        "T1 vs T2 (the two conditional-write realisations must be interchangeable)",
    )
    .await
    .expect("T1 and T2 must agree with each other, not just with the oracle");

    // Suppression proof: device 1 (unchanged effect) and device 3 (absent
    // from the delta) must keep their prior run's marker under BOTH
    // conditional techniques — proving the write never happened, not merely
    // that it reproduced the same bits.
    for table in ["t1_target", "t2_target"] {
        let rows = backend
            .execute_sql(&format!(
                "SELECT device_id, run_marker FROM main.{table} ORDER BY device_id"
            ))
            .await
            .expect("read back marker column");
        let batch = &rows[0];
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .expect("device_id is Int64");
        let markers = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        let by_id: std::collections::HashMap<i64, String> = (0..ids.len())
            .map(|i| (ids.value(i), markers.value(i).to_string()))
            .collect();
        assert_eq!(
            by_id.get(&1).map(String::as_str),
            Some("run1"),
            "{table}: device 1's unchanged-effect row must never be written"
        );
        assert_eq!(
            by_id.get(&2).map(String::as_str),
            Some("run2"),
            "{table}: device 2's changed row must be written"
        );
        assert_eq!(
            by_id.get(&3).map(String::as_str),
            Some("run1"),
            "{table}: device 3 (absent from the delta) must never be touched"
        );
        assert_eq!(
            by_id.get(&4).map(String::as_str),
            Some("run2"),
            "{table}: device 4 (brand new) must be inserted with the new run's marker"
        );
    }
}

// =============================================================================
// Phase E4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
// — delta-restriction (T3) vs widened-scan equivalence at a fixed
// processed-input set `S`, plus the empty-delta no-op cascade end to end.
// Both legs exercise REAL production entry points directly
// (`append_model_edge_cells`, `execute_delete_insert_with_delta_
// restriction`, `execute_column_scoped_merge_full`,
// `plan_since_upstream_with_observed_deltas`) rather than a hand-rolled
// reimplementation — the same "direct fact injection" discipline
// `crates/smelt-runtime/tests/delta_restricted_recompute.rs` (E3) and
// `crates/smelt-runtime/tests/since_upstream_propagation.rs`'s D3 tests
// already use, generalized to a generated sample.
// =============================================================================

/// Total baseline keys per generated case for `EnrichmentEdgeRecipe` —
/// `arb_enrichment_edge_schedule`'s own `1..total` non-empty-proper-subset
/// contract needs `total >= 2`.
const ENRICHMENT_TOTAL_KEYS: usize = 6;

const ENRICHMENT_DEFAULT_CASES: usize = 12;

fn enrichment_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ENRICHMENT_DEFAULT_CASES)
}

/// Derive the REAL P1 skeleton-source-closure verdict for a
/// `EnrichmentJoinKind`'s own model-edge scope, through the SAME production
/// entry point `crates/smelt-runtime/tests/
/// web_analytics_session_delta_restriction.rs` exercises
/// (`append_model_edge_cells`), never a hand-typed classification.
fn enrichment_edge_closed(join_kind: EnrichmentJoinKind) -> bool {
    let recipe = smelt_maintenance_testkit::recipe::EnrichmentEdgeRecipe::new(join_kind);
    let mut plan = smelt_logical::maintenance::MaintenancePlan::default();
    smelt_logical::maintenance::derive::append_model_edge_cells(
        &mut plan,
        &recipe.model_body(),
        Some("event_date"),
        &recipe.model_edges(),
        &[],
    );
    let cell = plan
        .cell_for(&Trigger::NewData {
            source: recipe.driving_source().to_string(),
        })
        .unwrap_or_else(|| panic!("{join_kind:?} produced no model-edge creation cell"));
    cell.skeleton_source_closure
        .as_ref()
        .is_some_and(|c| c.is_closed())
}

/// Seed `main.<table>` with [`ENRICHMENT_TOTAL_KEYS`] baseline rows shaped
/// like `web_analytics_session_delta_restriction.rs`'s own
/// `events_enriched` fixture; a key in `schedule.touched_indices` gets its
/// `event_utm_campaign` value suffixed by `touched_suffix` (empty for a
/// plain baseline, `"-NEW"` for the recompute source that actually changed).
async fn seed_enrichment_case(
    backend: &DuckDbBackend,
    table: &str,
    schedule: &smelt_maintenance_testkit::recipe::EnrichmentEdgeSchedule,
    touched_suffix: &str,
) {
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.{table} (event_id VARCHAR, device_id VARCHAR, event_date DATE, \
             event_utm_campaign VARCHAR, session_id VARCHAR, session_utm_campaign VARCHAR)"
        ))
        .await
        .unwrap();
    let rows: Vec<String> = (0..ENRICHMENT_TOTAL_KEYS)
        .map(|k| {
            let campaign = if schedule.touched_indices.contains(&k) {
                format!("campaign-{k}{touched_suffix}")
            } else {
                format!("campaign-{k}")
            };
            format!("('ev-{k}', 'dev-{k}', '2026-07-01', '{campaign}', 'sess-{k}', 'campaign-{k}')")
        })
        .collect();
    backend
        .execute_sql(&format!(
            "INSERT INTO main.{table} VALUES {}",
            rows.join(", ")
        ))
        .await
        .unwrap();
}

async fn read_enrichment_rows(backend: &DuckDbBackend, table: &str) -> Vec<(String, String)> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT event_id, event_utm_campaign FROM main.{table} ORDER BY event_id"
        ))
        .await
        .unwrap();
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let campaigns = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            out.push((ids.value(i).to_string(), campaigns.value(i).to_string()));
        }
    }
    out
}

async fn record_enrichment_delta(
    backend: &DuckDbBackend,
    upstream: &str,
    start: &str,
    end: &str,
    changed_keys: &[String],
) {
    let ensure = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend.execute_sql(&ensure).await.unwrap();
    let changed_keys_query = if changed_keys.is_empty() {
        "SELECT NULL AS delta_key, NULL AS delta_partition WHERE FALSE".to_string()
    } else {
        let keys_list = changed_keys
            .iter()
            .map(|k| format!("('{k}', NULL)"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT * FROM (VALUES {keys_list}) AS t(delta_key, delta_partition)")
    };
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        upstream,
        start,
        end,
        &changed_keys_query,
    );
    backend.execute_sql(&upsert).await.unwrap();
}

/// `delta_restricted_equals_widened_scan_at_fixed_s` (Phase E4 TDD list):
/// over the closure-admitted subset of a generated `EnrichmentEdgeRecipe`
/// sample, force the SAME schedule through both `execute_delete_insert_
/// with_delta_restriction` dispatch outcomes — `Closed` (restricted) and a
/// forced `Open` (widened) — against two independently-seeded-identical
/// baselines, and assert bit-identical end state. This holds precisely
/// because every key OUTSIDE the schedule's touched set carries a
/// recompute-source value already identical to what is stored — recomputing
/// it (widened) reproduces exactly what leaving it alone (restricted)
/// would.
#[tokio::test]
async fn delta_restricted_equals_widened_scan_at_fixed_s() {
    let n = enrichment_case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_enrichment_edge_recipe();
    let schedule_strat = arb_enrichment_edge_schedule(ENRICHMENT_TOTAL_KEYS);

    let mut admitted = 0;
    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();

        let closed = enrichment_edge_closed(recipe.join_kind);
        assert_eq!(
            closed,
            recipe.expects_closed(),
            "case {i}: {recipe:?} P1 verdict ({closed}) did not match the recipe's own \
             closure-admissibility expectation"
        );
        if !closed {
            continue;
        }
        admitted += 1;

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("db.duckdb");
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");

        let untouched_baseline = smelt_maintenance_testkit::recipe::EnrichmentEdgeSchedule {
            touched_indices: vec![],
        };
        seed_enrichment_case(&backend, "restricted_target", &untouched_baseline, "").await;
        seed_enrichment_case(&backend, "widened_target", &untouched_baseline, "").await;
        seed_enrichment_case(&backend, "enrichment_recompute", &schedule, "-NEW").await;

        let changed_keys: Vec<String> = schedule
            .touched_indices
            .iter()
            .map(|k| format!("ev-{k}"))
            .collect();
        record_enrichment_delta(
            &backend,
            recipe.driving_source(),
            "2026-07-01",
            "2026-07-02",
            &changed_keys,
        )
        .await;

        let region = smelt_logical::maintenance::emit::Region {
            start: "'2026-07-01'".to_string(),
            end: "'2026-07-02'".to_string(),
        };
        let body = "SELECT event_id, device_id, event_date, event_utm_campaign, session_id, \
                     session_utm_campaign FROM main.enrichment_recompute";

        let closed_verdict = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
            row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
        };
        smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
            &backend,
            "main",
            "restricted_target",
            "event_date",
            &region,
            body,
            Some("event_id"),
            Some(&closed_verdict),
            recipe.driving_source(),
            "2026-07-01",
            "2026-07-02",
            smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: restricted recompute failed: {e}"));

        let open_verdict = smelt_logical::maintenance::SkeletonSourceClosure::Open {
            reason: "forced widened-scan comparison".to_string(),
        };
        smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
            &backend,
            "main",
            "widened_target",
            "event_date",
            &region,
            body,
            Some("event_id"),
            Some(&open_verdict),
            recipe.driving_source(),
            "2026-07-01",
            "2026-07-02",
            smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
            &no_retry_policy(),
            &smelt_runtime::probes::ProbePolicy::per_run(),
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: widened recompute failed: {e}"));

        let restricted_rows = read_enrichment_rows(&backend, "restricted_target").await;
        let widened_rows = read_enrichment_rows(&backend, "widened_target").await;
        assert_eq!(
            restricted_rows, widened_rows,
            "case {i}: {recipe:?} schedule {schedule:?} — delta-restricted and widened-scan \
             recomputes must be bit-identical at fixed S"
        );
    }

    assert!(
        admitted > 0,
        "N={n} deterministic sample admitted zero closure-Closed cases — generator/proof \
         regression"
    );
}

/// `delta_restriction_admission_rate_stays_above_floor` (Phase E4 TDD list):
/// exactly one of the three `EnrichmentJoinKind` variants (`LeftJoin`) is
/// closure-admissible by construction, so a uniform draw over N=30 should
/// land close to 33%; a 15% floor catches a generator or P1 regression
/// (`InnerJoin`/`MembershipPredicate` spuriously admitting, or `LeftJoin`
/// spuriously refusing) with wide margin against sampling noise, without
/// being flaky (`TestRunner::deterministic()` reproduces the SAME sequence
/// every run).
#[test]
fn delta_restriction_admission_rate_stays_above_floor() {
    const N: usize = 30;
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_enrichment_edge_recipe();

    let mut admitted = 0;
    for _ in 0..N {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        if enrichment_edge_closed(recipe.join_kind) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.15,
        "delta-restriction admission rate {rate:.2} over N={N} fell below the 15% floor \
         ({admitted}/{N} admitted) — a route-admission regression would silently hollow out the \
         standing gate"
    );
}

// =============================================================================
// `empty_delta_cascade_is_a_no_op` (Phase E4): the end-to-end payoff — a
// fully-suppressed conditional write over a composed (timeseries-
// partitioned) model-edge upstream records a REAL present-and-empty
// observed delta (T5, via the real `execute_column_scoped_merge_full`
// entry point, not a hand-typed record), which schedules ZERO downstream
// regions across a real fan-out cascade (`examples/timeseries`'s real
// `user_daily_spend -> {user_spend_rollup, user_spend_running_total}`
// graph, the same real fixture `crates/smelt-runtime/tests/
// since_upstream_propagation.rs`'s D3 tests exercise), and leaves the
// target byte-identical to a from-scratch full-refresh oracle.
// =============================================================================

fn key_suppression_for(compared: &[&str]) -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: compared.iter().map(|c| c.to_string()).collect(),
    }
}

async fn read_spend_rows(backend: &DuckDbBackend, table: &str) -> Vec<(i64, String, f64)> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT user_id, spend_date::VARCHAR, total_amount FROM main.{table} ORDER BY user_id"
        ))
        .await
        .unwrap();
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, Float64Array, Int64Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let dates = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let amounts = batch
            .column(2)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for i in 0..batch.num_rows() {
            out.push((ids.value(i), dates.value(i).to_string(), amounts.value(i)));
        }
    }
    out
}

#[tokio::test]
async fn empty_delta_cascade_is_a_no_op() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("db.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    // The upstream: `user_daily_spend`-styled (`examples/timeseries`'s own
    // model name and columns), already processed for 2026-07-01.
    backend
        .execute_sql(
            "CREATE TABLE main.user_daily_spend (user_id BIGINT, spend_date DATE, total_amount \
             DOUBLE)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.user_daily_spend VALUES (1, '2026-07-01', 10.0), \
             (2, '2026-07-01', 20.0), (3, '2026-07-01', 30.0)",
        )
        .await
        .unwrap();

    // A redelivery of the SAME window: byte-identical to what is stored —
    // an upstream run that changes nothing.
    backend
        .execute_sql(
            "CREATE TABLE main.user_daily_spend_recompute (user_id BIGINT, spend_date DATE, \
             total_amount DOUBLE)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.user_daily_spend_recompute VALUES (1, '2026-07-01', 10.0), \
             (2, '2026-07-01', 20.0), (3, '2026-07-01', 30.0)",
        )
        .await
        .unwrap();

    let suppression = key_suppression_for(&["total_amount"]);
    let dimension_batch_sql =
        "SELECT user_id, spend_date, total_amount FROM main.user_daily_spend_recompute";
    let window = smelt_backend::PartitionRange {
        column: "spend_date".to_string(),
        start: "2026-07-01".to_string(),
        end: "2026-07-02".to_string(),
    };

    // Leg (a): the write itself, executed for real — snapshot the target
    // before, run the real conditional write + record entry point, snapshot
    // after. Zero writes, not merely zero net diffs.
    let before = read_spend_rows(&backend, "user_daily_spend").await;
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "user_daily_spend",
        &["user_id".to_string()],
        dimension_batch_sql,
        &suppression,
        &window,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed merge over an unchanged redelivery must succeed");
    let after = read_spend_rows(&backend, "user_daily_spend").await;
    assert_eq!(
        before, after,
        "an unchanged redelivery must write zero rows — the target's state must be \
         byte-identical before and after"
    );

    // The REAL recorded delta (T5) — read back through the same production
    // entry point `crates/smelt-runtime/tests/observed_delta.rs` exercises,
    // never hand-typed.
    let changed_keys = smelt_runtime::maintenance_driver::read_observed_delta_changed_keys(
        &backend,
        "main",
        "user_daily_spend",
        "2026-07-01",
        "2026-07-02",
    )
    .await
    .expect("read observed delta")
    .expect("a fully-suppressed run must record a present (not absent) delta");
    assert!(
        changed_keys.is_empty(),
        "a fully-suppressed run must record an EMPTY changed-key set: {changed_keys:?}"
    );

    // Leg (c): the full-refresh oracle — an independent, from-scratch
    // recompute over the SAME (unchanged) source data — still matches.
    backend
        .execute_sql(
            "CREATE TABLE main.oracle_daily_spend AS SELECT user_id, spend_date, total_amount \
             FROM main.user_daily_spend_recompute",
        )
        .await
        .unwrap();
    let mut sorted_after = after.clone();
    sorted_after.sort_by_key(|r| r.0);
    let mut sorted_oracle = read_spend_rows(&backend, "oracle_daily_spend").await;
    sorted_oracle.sort_by_key(|r| r.0);
    assert_eq!(
        sorted_after, sorted_oracle,
        "the no-op run's end state must still equal a from-scratch full-refresh oracle"
    );

    // Leg (b): the REAL propagation graph — `examples/timeseries`'s actual
    // `user_daily_spend -> {user_spend_rollup, user_spend_running_total}`
    // fan-out — feeding the delta this test JUST recorded for real must
    // schedule ZERO regions across the WHOLE cascade, not just its own
    // edge.
    let project_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    let discovery =
        smelt_core::ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = smelt_core::discover_source_infos(&project_dir, &["models".to_string()]);

    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| {
            a == "user_daily_spend" || a == "user_spend_rollup" || a == "user_spend_running_total"
        })
        .collect();
    assert_eq!(
        order.len(),
        3,
        "expected all three real fixture models to be discovered: {order:?}"
    );

    let window_interval = smelt_logical::maintenance::propagate::DayInterval::new(
        smelt_logical::maintenance::propagate::day_ordinal(2026, 7, 1),
        smelt_logical::maintenance::propagate::day_ordinal(2026, 7, 2),
    );
    let deltas = vec![smelt_runtime::propagation::SourceDelta {
        source: "user_daily_spend".to_string(),
        landed: window_interval,
    }];
    let mut observed = smelt_runtime::propagation::ObservedDeltaLookup::new();
    observed.insert(
        (
            "user_daily_spend".to_string(),
            "2026-07-01".to_string(),
            "2026-07-02".to_string(),
        ),
        smelt_state::ddl_duckdb::ObservedDelta {
            changed_keys,
            partitions: vec![],
        },
    );

    let plan = smelt_runtime::propagation::plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &observed,
    )
    .expect("a present-and-empty observed delta must not be a refusal");

    assert!(
        plan.runs.is_empty(),
        "a fully-suppressed upstream run must schedule ZERO downstream regions across the whole \
         cascade (both user_spend_rollup and user_spend_running_total): {:?}",
        plan.runs
    );
}
