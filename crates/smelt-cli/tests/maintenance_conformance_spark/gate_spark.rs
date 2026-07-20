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
    arb_recipe, ConformanceTarget, ModelRecipe, RecipePool, SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_schedule_for, read_source_snapshot_via_backend, ConformanceSchedule, ConformanceStep,
    GenRow,
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
    tracker.materialize_s_as_view(backend, k).await?;
    let maintained_sql = format!(
        "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
        recipe.model_name
    );
    let oracle_sql = tracker.s_restricted_oracle_sql(recipe);
    let equal = multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "Spark S-restricted equivalence violated for model {:?} at run {k}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
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
                assert_equivalence_spark(recipe, backend.as_ref(), &tracker, k).await?;
            }
            ConformanceStep::AppendLateRow(row) => {
                insert_row(backend.as_ref(), recipe, row).await?;
            }
            other => anyhow::bail!(
                "unsupported ConformanceStep for the Spark leg (Phase 3 scope is \
                 RunWindow/AppendLateRow only, matching arb_schedule_for's own generator): \
                 {other:?}"
            ),
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
