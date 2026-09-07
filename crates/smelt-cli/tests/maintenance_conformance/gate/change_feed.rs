//! The `SimulatedChangeFeed` step family: recompute-only admission for `change_feed`-declared sources, and equivalence via that recompute.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::keyed_oracle::classify_keyed_full;
use super::mixed_pool::insert_fact_row;
use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::feed::{self, FeedSourcePosture};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{arb_keyed_combiner, MutableEnrichedRecipe};
use smelt_maintenance_testkit::schedule_gen::GenRow;

// ---------------------------------------------------------------------
// Phase 8: the `SimulatedChangeFeed` step family — recompute-only
// admission for `change_feed`-declared sources
// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 8;
// `incremental_models.md` §Known Divergences' `change_feed`-scoping entry).
// ---------------------------------------------------------------------

/// Default deterministic case count for `change_feed_source_admits_recompute_only`.
pub(crate) const FEED_ADMISSION_DEFAULT_CASES: usize = 10;

pub(crate) fn feed_admission_case_count() -> usize {
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
pub(crate) const FEED_RECOMPUTE_DEFAULT_CASES: usize = 6;

pub(crate) fn feed_recompute_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_FEED_RECOMPUTE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FEED_RECOMPUTE_DEFAULT_CASES)
}

/// Generous upper bound on distinct dimension ids this test's fact rows
/// ever reference — mirrors `MIXED_DIM_SEED_MAX_ID` (Phase 4's own
/// convention above).
pub(crate) const FEED_DIM_SEED_MAX_ID: i64 = 12;

/// Wrap `sql` restricted to the single partition day `day` on column
/// `day_col` — used to isolate one window's own rows so a genuinely
/// incremental run's freshness (or an older, un-revisited window's frozen
/// staleness) can be checked without the whole-table noise of every other
/// window.
pub(crate) fn restrict_to_day(sql: &str, day: chrono::NaiveDate, day_col: &str) -> String {
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
                    val: Some(id * 10),
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
                        val: Some(dim_id * 10 + step_i as i64),
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
