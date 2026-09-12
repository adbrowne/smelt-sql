#![cfg(feature = "duckdb")]
//! Replay harness for `examples/github_activity/`
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/02-plan.md`).
//!
//! Asserts, over a short (7-day) slice of the committed Parquet fixture, the
//! properties the phase 2 plan's "Tests (red first)" section names:
//!
//!   - the loader's deliberate previous-day redelivery actually lands
//!     duplicate physical rows in `raw.github_events`, and
//!     `silver.events_deduped` emits exactly `count(DISTINCT id)` rows;
//!   - the declared `key_recurrence` bound on `raw.github_events` is
//!     genuinely checked, not decorative: a duplicate pair that violates it
//!     fails the run transactionally (`KeyedRecurrenceBoundViolated`) rather
//!     than silently mis-counting — the negative control the plan asks for.
//!     (`silver.events_deduped`'s dedup itself is a keyed `MERGE`, which is
//!     idempotent regardless of window width, so there is no SQL-derived
//!     lookback to narrow the way `events_parsed`'s arrival-based filter has
//!     in `examples/web_analytics/` — see that model's own comment for why.)
//!   - a session spanning a UTC midnight stays one session, not two;
//!   - full-refresh and the day-by-day incremental replay agree on every
//!     model, over the full 30-day fixture range.
//!
//! 7 days keeps CI runtime bounded; the full-refresh equivalence test uses
//! the entire 30-day fixture since that is what the outcome's criterion 7
//! (DuckDB half) actually promises.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

mod github_activity_support;
use github_activity_support::{
    create_empty_raw_table, day_after, duckdb_exec, duckdb_scalar_i64, load_day, replay_days,
    smelt_bin, smelt_run, stage_workspace, FIXTURE_DAYS,
};

fn smelt_run_expect_failure(workspace: &Path, start: &str, end: &str) -> String {
    let out = Command::new(smelt_bin())
        .args(["run", "--event-time-start", start, "--event-time-end", end])
        .current_dir(workspace)
        .env("RUST_LOG", "warn")
        // See `smelt_run`'s comment in `github_activity_support`: the staged
        // `smelt.yml`'s `databricks` target needs these resolvable at
        // config-load time regardless of the selected target.
        .env("SMELT_DBX_HOST", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_HOSTNAME", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt run: {e}"));
    assert!(
        !out.status.success(),
        "expected smelt run [{start} .. {end}) to fail, but it succeeded\nstdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The loader's redelivery lands physical duplicates, and the keyed dedup
/// collapses them to exactly `count(DISTINCT id)` — the plan's headline
/// assertion (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/
/// 02-plan.md` §"Tests (red first)").
#[test]
fn redelivered_rows_land_and_dedup_collapses_to_distinct_ids() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, &FIXTURE_DAYS[0..7]);

    let raw_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.sources_raw_github_events");
    let distinct_ids = duckdb_scalar_i64(
        &db,
        "SELECT count(DISTINCT id) FROM main.sources_raw_github_events",
    );
    let deduped_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_events_deduped");

    assert!(
        raw_rows > distinct_ids,
        "expected the redelivery to produce physical duplicates in raw \
         (raw_rows={raw_rows}, distinct_ids={distinct_ids}) — a fixture that \
         never redelivers never exercises dedup"
    );
    assert_eq!(
        deduped_rows, distinct_ids,
        "silver.events_deduped must emit exactly count(DISTINCT id) rows \
         (raw_rows={raw_rows}, distinct_ids={distinct_ids}, deduped_rows={deduped_rows})"
    );
}

/// Negative control: `raw.github_events` declares `key_recurrence: {key:
/// [id], window: '0 days'}` because a genuine redelivered duplicate always
/// shares its original's `created_at` exactly. A duplicate pair that
/// violates that bound (here: same `id`, `created_at` shifted by a day) must
/// fail the run transactionally rather than silently mis-dedup — proving the
/// declared bound is checked, not decorative
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality").
#[test]
fn recurrence_bound_violation_fails_the_run() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    create_empty_raw_table(&db, &sample);
    load_day(&db, &sample, "2026-08-05", None);
    smelt_run(&workspace, "2026-08-05", "2026-08-06", &[]);

    // Inject a corrupt "redelivery": same id as a real 2026-08-05 row, but
    // shifted a day later on the event-time axis — violating the declared
    // zero-width recurrence bound.
    duckdb_exec(
        &db,
        &format!(
            "INSERT INTO main.sources_raw_github_events \
             SELECT id, type, created_at + INTERVAL '1 day', actor_id, actor_login, \
                    repo_id, repo_name, org_id, public, payload \
             FROM read_parquet('{}') \
             WHERE CAST(created_at AS DATE) = DATE '2026-08-05' \
             LIMIT 1;",
            sample.display()
        ),
    );
    load_day(&db, &sample, "2026-08-06", None);

    let output = smelt_run_expect_failure(&workspace, "2026-08-06", "2026-08-07");
    assert!(
        output.contains("KeyedRecurrenceBoundViolated") || output.contains("RecurrenceBound"),
        "expected a recurrence-bound violation diagnostic, got:\n{output}"
    );
}

/// A session whose events straddle a UTC midnight stays one session (the
/// clock-anchored cut, `functions/sessionize.sql`), not two.
#[test]
fn session_spanning_midnight_stays_one_session() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, &FIXTURE_DAYS[0..3]);

    // Any actor with events on both sides of a midnight, less than 30
    // minutes apart, must appear as exactly one session spanning that
    // boundary rather than two sessions ending/starting at the boundary.
    let violations = duckdb_scalar_i64(
        &db,
        "SELECT count(*)
         FROM main.silver_actor_sessions s1
         JOIN main.silver_actor_sessions s2
           ON s1.actor_id = s2.actor_id
          AND s1.session_start_ts < s2.session_start_ts
          AND epoch_us(s2.session_start) - epoch_us(s1.session_end) < 30 * 60 * 1000000",
    );
    assert_eq!(
        violations, 0,
        "found adjacent same-actor sessions within the 30-minute gap that should have merged \
         into one clock-anchored session"
    );
}

/// Full-refresh and the day-by-day incremental replay agree on every model,
/// over the full 30-day fixture — criterion 7's DuckDB half
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`).
#[test]
fn full_refresh_matches_incremental_replay() {
    let tmp = TempDir::new().expect("tempdir");

    let (incr_workspace, incr_db, incr_sample) = stage_workspace(&tmp.path().join("incremental"));
    replay_days(&incr_workspace, &incr_db, &incr_sample, FIXTURE_DAYS);

    let (full_workspace, full_db, full_sample) = stage_workspace(&tmp.path().join("full"));
    create_empty_raw_table(&full_db, &full_sample);
    duckdb_exec(
        &full_db,
        &format!(
            "INSERT INTO main.sources_raw_github_events SELECT * FROM read_parquet('{}'); \
             INSERT INTO main.sources_raw_github_events_arrival \
             SELECT *, CAST(created_at AS DATE) AS ingested_date FROM read_parquet('{}');",
            full_sample.display(),
            full_sample.display()
        ),
    );
    smelt_run(
        &full_workspace,
        "2026-08-05",
        "2026-09-04",
        &["--full-refresh"],
    );

    // Per-window, row-for-row equivalence over every materialised relation
    // (including the `silver.repo_naming`/`silver.actor_naming` succession
    // ties, the `gold.events_enriched` enrichment staleness, and the
    // `silver.actor_sessions`/`marts.daily_active_contributors` oracle
    // windowing gap — each characterised by a bounded registry entry rather
    // than a magic row-count delta) is `github_activity_oracle.rs`'s job
    // (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/06-plan.md`,
    // `phases/08-plan.md`); its `every_window_matches_the_full_refresh_
    // oracle` supersedes every row-count assertion this test used to make
    // per-table, so they are retired here rather than duplicated.
    let incr_deduped = duckdb_scalar_i64(
        &incr_db,
        "SELECT count(DISTINCT id) FROM main.silver_events_deduped",
    );
    let full_deduped = duckdb_scalar_i64(
        &full_db,
        "SELECT count(DISTINCT id) FROM main.silver_events_deduped",
    );
    assert_eq!(incr_deduped, full_deduped);
    assert_eq!(
        incr_deduped, 64_313,
        "expected the full fixture to dedup to the measured 64,313 distinct ids \
         (docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md)"
    );
}

/// Run `smelt explain <model> --project-dir <workspace>` against a staged (not
/// yet built) workspace and return stdout as text.
fn smelt_explain(workspace: &Path, model: &str) -> String {
    let out = Command::new(smelt_bin())
        .args(["explain", model, "--project-dir"])
        .arg(workspace)
        .env("RUST_LOG", "warn")
        .env("SMELT_DBX_HOST", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_HOSTNAME", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt explain: {e}"));
    assert!(
        out.status.success(),
        "smelt explain {model} failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Test 1 (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/03-plan.md`):
/// `silver.repo_naming` is recognised as the succession grain from its SQL
/// shape alone, driven by the event-time-partitioned source.
#[test]
fn repo_naming_is_recognised_as_the_succession_grain() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, _db, _sample) = stage_workspace(tmp.path());
    let report = smelt_explain(&workspace, "silver.repo_naming");

    assert!(
        report.contains("grain: succession"),
        "expected `grain: succession`: {report}"
    );
    assert!(
        report.contains("identity: (repo_id, created_at)"),
        "expected `identity: (repo_id, created_at)`: {report}"
    );
    assert!(
        report.contains("technique: succession-patch"),
        "expected `technique: succession-patch`: {report}"
    );
    assert!(
        report.contains("run axis: created_at (event-time-partitioned)"),
        "expected the event-time-partitioned run axis line: {report}"
    );
    assert!(
        report.contains("clock: created_at"),
        "expected the clock line: {report}"
    );
}

/// Test 2: `silver.actor_naming` is the same grain, driven by the
/// arrival-partitioned twin source.
#[test]
fn actor_naming_is_arrival_partitioned() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, _db, _sample) = stage_workspace(tmp.path());
    let report = smelt_explain(&workspace, "silver.actor_naming");

    assert!(
        report.contains("run axis: ingested_date (arrival-partitioned)"),
        "expected the arrival-partitioned run axis line: {report}"
    );
    assert!(
        report.contains("clock: created_at"),
        "expected the clock line: {report}"
    );
}

/// Test 3: after the full 30-day day-by-day replay, both succession models
/// have exactly `count(DISTINCT (key, created_at))` rows over their driving
/// relation — the loader's deliberate previous-day redelivery folds once
/// rather than duplicating history, and neither run fails with
/// `SuccessionClockTie` or `SourceMutationProfileViolated`.
#[test]
fn redelivery_folds_once_in_both_succession_models() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let repo_naming_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_repo_naming");
    let repo_distinct = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM (SELECT DISTINCT repo_id, created_at FROM \
         main.sources_raw_github_events)",
    );
    assert_eq!(
        repo_naming_rows, repo_distinct,
        "repo_naming must fold the redelivery to exactly \
         count(DISTINCT (repo_id, created_at)) (rows={repo_naming_rows}, \
         distinct={repo_distinct})"
    );

    let actor_naming_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_actor_naming");
    let actor_distinct = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM (SELECT DISTINCT actor_id, created_at FROM \
         main.sources_raw_github_events_arrival)",
    );
    assert_eq!(
        actor_naming_rows, actor_distinct,
        "actor_naming must fold the redelivery to exactly \
         count(DISTINCT (actor_id, created_at)) (rows={actor_naming_rows}, \
         distinct={actor_distinct})"
    );
}

/// Test 4: the fixture's own same-second ties (139 `(repo_id, created_at)`,
/// 145 `(actor_id, created_at)` — measured, `docs/outcomes/
/// 20260906-bigquery-dogfood-spine/outcome.md` decision log) exist in the raw
/// sample and, since every tied pair agrees on the projected name, fold once
/// exactly like a redelivery rather than raising `SuccessionClockTie`.
#[test]
fn same_second_events_fold_once_within_a_key() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());

    // "Ties" here counts extra rows beyond the first per key — count(*) minus
    // count(DISTINCT (key, created_at)) — matching how the outcome's decision
    // log measured 139/145 (not the number of tied groups).
    let repo_tie_count = {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        conn.query_row(
            &format!(
                "SELECT count(*) - count(DISTINCT (repo_id, created_at)) \
                 FROM read_parquet('{}')",
                sample.display()
            ),
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("query repo ties")
    };
    let actor_tie_count = {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        conn.query_row(
            &format!(
                "SELECT count(*) - count(DISTINCT (actor_id, created_at)) \
                 FROM read_parquet('{}')",
                sample.display()
            ),
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("query actor ties")
    };
    assert_eq!(
        repo_tie_count, 139,
        "expected the fixture's measured 139 same-second (repo_id, created_at) ties"
    );
    assert_eq!(
        actor_tie_count, 145,
        "expected the fixture's measured 145 same-second (actor_id, created_at) ties"
    );

    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    // No tie failure was raised (`replay_days` would have panicked via
    // `smelt_run`'s success assertion), and the ties collapsed into the
    // fold-once row counts already proven by
    // `redelivery_folds_once_in_both_succession_models`.
    let repo_naming_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_repo_naming");
    let repo_distinct = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM (SELECT DISTINCT repo_id, created_at FROM \
         main.sources_raw_github_events)",
    );
    assert_eq!(repo_naming_rows, repo_distinct);
}

/// Test 5: `marts.naming_history` surfaces the real renames — 34 distinct
/// renamed `repo_id`s, 4 renamed `actor_id`s, the owner-change row, and the 2
/// reused repo names each appearing under both `repo_id`s (`docs/outcomes/
/// 20260906-bigquery-dogfood-spine/outcome.md` decision log).
#[test]
fn naming_history_surfaces_the_real_renames() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let renamed_repos = duckdb_scalar_i64(
        &db,
        "SELECT count(DISTINCT entity_id) FROM main.marts_naming_history WHERE entity_kind = \
         'repo'",
    );
    assert_eq!(
        renamed_repos, 34,
        "expected 34 distinct renamed repo_ids in naming_history"
    );

    let renamed_actors = duckdb_scalar_i64(
        &db,
        "SELECT count(DISTINCT entity_id) FROM main.marts_naming_history WHERE entity_kind = \
         'actor'",
    );
    assert_eq!(
        renamed_actors, 4,
        "expected 4 distinct renamed actor_ids in naming_history"
    );

    let owner_change_row = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM main.marts_naming_history \
         WHERE entity_kind = 'repo' AND from_name = 'mikiKG45/noob-devops-project' \
         AND to_name = 'guslariR45/noob-devops-project'",
    );
    assert_eq!(
        owner_change_row, 1,
        "expected the owner-change rename row (same repo_id, different owner) to appear"
    );

    // The 2 repo names reused across different repo_ids each appear under
    // both ids somewhere in the naming stream (as either a from_name, a
    // to_name, or the current name in silver.repo_naming).
    let reused_names = duckdb_scalar_i64(
        &db,
        "WITH names AS (
            SELECT repo_id, repo_name AS name FROM main.silver_repo_naming
            UNION
            SELECT entity_id AS repo_id, from_name AS name FROM main.marts_naming_history \
         WHERE entity_kind = 'repo'
            UNION
            SELECT entity_id AS repo_id, to_name AS name FROM main.marts_naming_history \
         WHERE entity_kind = 'repo'
         )
         SELECT count(*) FROM (
            SELECT name FROM names GROUP BY name HAVING count(DISTINCT repo_id) > 1
         )",
    );
    assert!(
        reused_names >= 2,
        "expected at least 2 repo names reused across different repo_ids, found {reused_names}"
    );
}

/// Run `smelt explain <model> --json --project-dir <workspace>` and parse
/// stdout as JSON.
fn smelt_explain_json(workspace: &Path, model: &str) -> serde_json::Value {
    let out = Command::new(smelt_bin())
        .args(["explain", model, "--json", "--project-dir"])
        .arg(workspace)
        .env("RUST_LOG", "warn")
        .env("SMELT_DBX_HOST", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_HOSTNAME", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt explain --json: {e}"));
    assert!(
        out.status.success(),
        "smelt explain {model} --json failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "smelt explain {model} --json produced invalid JSON: {e}\nstdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// Test (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/04-plan.md`):
/// `gold.repo_dim` has exactly one row per `repo_id` (5,016 in the fixture),
/// and a renamed repo's `current_repo_name` matches its `is_current` row in
/// `silver.repo_naming`.
#[test]
fn repo_dim_is_one_row_per_repo() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let repo_dim_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.gold_repo_dim");
    assert_eq!(
        repo_dim_rows, 5_016,
        "expected one row per repo in the fixture"
    );

    let distinct_repo_ids = duckdb_scalar_i64(
        &db,
        "SELECT count(DISTINCT repo_id) FROM main.gold_repo_dim",
    );
    assert_eq!(
        repo_dim_rows, distinct_repo_ids,
        "gold.repo_dim must have exactly one row per repo_id, no duplicates"
    );

    // The owner-change repo (`docs/outcomes/20260906-bigquery-dogfood-spine/
    // outcome.md` decision log): current name must match `silver.repo_naming`'s
    // `is_current` row, not an earlier name.
    let matches = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM main.gold_repo_dim d
         JOIN main.silver_repo_naming n ON d.repo_id = n.repo_id AND n.is_current
         WHERE d.repo_id = 1136323000 AND d.current_repo_name = n.repo_name",
    );
    assert_eq!(
        matches, 1,
        "gold.repo_dim's current_repo_name for the owner-change repo must match \
         silver.repo_naming's is_current row"
    );
}

/// Test: `gold.events_enriched` is row-preserving over `silver.events_deduped`
/// and every fact row's repo is present in the dimension.
#[test]
fn events_enriched_preserves_every_fact_row() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let deduped_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_events_deduped");
    let enriched_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.gold_events_enriched");
    assert_eq!(
        deduped_rows, enriched_rows,
        "the LEFT JOIN must preserve every fact row (deduped={deduped_rows}, \
         enriched={enriched_rows})"
    );

    let null_names = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM main.gold_events_enriched WHERE current_repo_name IS NULL",
    );
    assert_eq!(
        null_names, 0,
        "every fact repo must be present in gold.repo_dim — a NULL current_repo_name \
         means the dimension is missing a repo the fact table references"
    );
}

/// Test: for a repo the fixture renames, an *early* event (before the
/// rename) is enriched with the repo's CURRENT name, not the name in force
/// at that event — the property that makes the dimension join worth having.
#[test]
fn events_enriched_renamed_repo_carries_current_name() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    // repo_id 1322474000 renames from `Smpn4arjasa/dapuremmak` to
    // `dapuremmak/dapuremmak` at 2026-08-07 08:25:42 (measured). Its earliest
    // event (2026-08-07 06:11:35) predates the rename.
    let early_event_name = duckdb::Connection::open(&db)
        .unwrap_or_else(|e| panic!("open {db:?}: {e}"))
        .query_row(
            "SELECT current_repo_name FROM main.gold_events_enriched \
             WHERE repo_id = 1322474000 ORDER BY created_at ASC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("query early event's enriched name");
    assert_eq!(
        early_event_name, "dapuremmak/dapuremmak",
        "an early event for a renamed repo must carry the CURRENT name, not the \
         name in force at event time"
    );
}

/// Test (phase 5, `docs/outcomes/20260906-bigquery-correctness`): the
/// enrichment-keyed cell heals every already-written row, not merely the one
/// early event `events_enriched_renamed_repo_carries_current_name` samples —
/// after the full 30-day replay, every `gold_events_enriched` row's
/// `current_repo_name` equals its repo's CURRENT `gold_repo_dim` name,
/// including rows written days before that repo's rename ever happened.
#[test]
fn enrichment_heal_repairs_rows_written_before_the_rename() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let stale = duckdb::Connection::open(&db)
        .unwrap_or_else(|e| panic!("open {db:?}: {e}"))
        .query_row(
            "SELECT count(*) FROM main.gold_events_enriched e \
             JOIN main.gold_repo_dim d ON e.repo_id = d.repo_id \
             WHERE e.current_repo_name IS DISTINCT FROM d.current_repo_name",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("query stale row count");
    assert_eq!(
        stale, 0,
        "expected zero gold_events_enriched rows with a stale current_repo_name after the \
         full 30-day replay — the enrichment-keyed heal must dispatch every run and repair \
         every row, not just newly-written ones"
    );
}

/// Test: the `{current_repo_name}` `UpstreamMutation(gold.repo_dim)` cell's
/// resolved verdict, characterised exactly as `smelt explain --json`
/// produces it. `gold.repo_dim` is a clockless upstream MODEL feeding a
/// `grain: partition` downstream — a combination `append_model_edge_cells`'s
/// key-addressed route cannot admit (it needs the DOWNSTREAM's own declared
/// `unique_key`, which a `grain: partition` output has none of by
/// construction) — but the enrichment-keyed route now admits it instead: the
/// `LEFT JOIN ... ON f.repo_id = dim.repo_id` matches `gold.repo_dim`'s own
/// declared `unique_key`, `current_repo_name` is a pure value-enrichment
/// read (never in row-admission position), and `events_enriched`'s own
/// `maintenance.scan_bounds.per_source.gold.repo_dim.allow_full_scan: true`
/// accepts the full-table merge
/// (`docs/outcomes/20260906-bigquery-correctness/phases/04-plan.md`).
#[test]
fn events_enriched_dimension_mutation_cell_technique() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, _db, _sample) = stage_workspace(tmp.path());
    let json = smelt_explain_json(&workspace, "gold.events_enriched");

    let cells = json["cells"].as_array().expect("cells array");
    let repo_dim_cell = cells.iter().find(|c| {
        c["trigger"]
            .as_str()
            .unwrap_or("")
            .contains("gold.repo_dim")
    });
    let repo_dim_cell = repo_dim_cell
        .unwrap_or_else(|| panic!("expected a cell derived for gold.repo_dim: {cells:?}"));
    assert_eq!(
        repo_dim_cell["technique"].as_str(),
        Some("ColumnScopedMerge"),
        "expected the enrichment-keyed route's ColumnScopedMerge technique: {repo_dim_cell:?}"
    );
    assert_eq!(
        repo_dim_cell["group"].as_str(),
        Some("{current_repo_name}"),
        "expected the cell's group to name only the edge-provenanced column: {repo_dim_cell:?}"
    );

    let refusals = json["refusals"].as_array().expect("refusals array");
    let repo_dim_refusal = refusals
        .iter()
        .find(|r| r["text"].as_str().unwrap_or("").contains("gold.repo_dim"));
    assert!(
        repo_dim_refusal.is_none(),
        "expected no refusal naming gold.repo_dim now that the enrichment-keyed route \
         admits a cell for it: {refusals:?}"
    );
}

/// Test: `SUM(event_count)` over `gold.repo_activity_daily` equals
/// `silver.events_deduped`'s row count — every deduped event is counted
/// exactly once across the per-(repo, day) rollup.
#[test]
fn repo_activity_daily_totals_match_events() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let deduped_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_events_deduped");
    let activity_total = duckdb_scalar_i64(
        &db,
        "SELECT SUM(event_count) FROM main.gold_repo_activity_daily",
    );
    assert_eq!(
        activity_total, deduped_rows,
        "gold.repo_activity_daily's total event_count must equal \
         silver.events_deduped's row count"
    );
}

/// Test: `marts.star_growth`'s final cumulative value is exactly the
/// fixture's measured `WatchEvent` count (47), and the series is
/// non-decreasing.
#[test]
fn star_growth_counts_the_fixtures_watch_events() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let watch_events = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM main.gold_events_enriched WHERE type = 'WatchEvent'",
    );
    assert_eq!(
        watch_events, 47,
        "expected the fixture's measured 47 WatchEvents"
    );

    let final_cumulative = duckdb_scalar_i64(
        &db,
        "SELECT cumulative_stars FROM main.marts_star_growth ORDER BY event_date DESC LIMIT 1",
    );
    assert_eq!(
        final_cumulative, 47,
        "marts.star_growth's final cumulative value must equal the total WatchEvent count"
    );

    let non_monotonic = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM (
            SELECT cumulative_stars,
                   LAG(cumulative_stars) OVER (ORDER BY event_date) AS prev
            FROM main.marts_star_growth
         ) WHERE prev IS NOT NULL AND cumulative_stars < prev",
    );
    assert_eq!(non_monotonic, 0, "marts.star_growth must be non-decreasing");
}

/// Test: `marts.repo_leaderboard`'s top row reproduces the sample's
/// documented skew rather than hiding it — the top repo is the fixture's
/// known highest-event-count repo (measured: repo_id 1331137000,
/// `mosleyamanda283/eltuxy`, 1,750 events).
#[test]
fn repo_leaderboard_top_repo_is_the_bot_repo() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let conn = duckdb::Connection::open(&db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    let (top_repo_id, top_repo_name, top_events): (i64, String, i64) = conn
        .query_row(
            "SELECT repo_id, current_repo_name, total_events FROM main.marts_repo_leaderboard \
             ORDER BY total_events DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("query top leaderboard row");

    assert_eq!(
        top_repo_id, 1_331_137_000,
        "expected the known highest-event-count repo"
    );
    assert_eq!(top_repo_name, "mosleyamanda283/eltuxy");
    assert_eq!(
        top_events, 1_750,
        "expected the fixture's measured top event count"
    );
}

/// Criterion 3/4 on a real fixture, not a synthetic scaffold: a staged
/// workspace with **no** pre-loaded rows — no `load_day.sh` call, no
/// `create_empty_raw_table` — ends with the first fixture day's rows in
/// both raw sources and `bronze.events` built, because `smelt run` itself
/// invokes `models/sources/raw/github_loader.yml`'s declared external step
/// ahead of every model that reads either source
/// (`docs/outcomes/20260906-external-dag-steps/phases/07-plan.md` task 4).
#[test]
fn smelt_run_invokes_the_loader_step() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, _sample) = stage_workspace(tmp.path());

    let day = FIXTURE_DAYS[0];
    smelt_run(&workspace, day, &day_after(day), &[]);

    let raw_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.sources_raw_github_events");
    let bronze_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.bronze_events");
    assert!(
        raw_rows > 0,
        "smelt run should have driven the loader step, populating raw.github_events"
    );
    assert_eq!(
        raw_rows, bronze_rows,
        "bronze.events is a passthrough of the source the step loaded"
    );
}

/// Criterion 4 on a real fixture: replacing `load_day.sh` with an `exit 3`
/// stub makes `smelt run` fail non-zero, naming the step, with no model
/// relation created (`docs/outcomes/20260906-external-dag-steps/phases/
/// 07-plan.md` task 4).
#[test]
fn loader_step_failure_leaves_downstream_unbuilt() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, _sample) = stage_workspace(tmp.path());
    std::fs::write(
        workspace.join("load_day.sh"),
        "#!/usr/bin/env bash\nexit 3\n",
    )
    .expect("overwrite load_day.sh with a failing stub");

    let day = FIXTURE_DAYS[0];
    let output = smelt_run_expect_failure(&workspace, day, &day_after(day));
    assert!(
        output.contains("sources.raw.github_loader") || output.contains("ExternalStepFailed"),
        "expected the run failure to name the failing step:\n{output}"
    );

    let conn = duckdb::Connection::open(&db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    let table_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_name = 'bronze_events'",
            [],
            |row| row.get(0),
        )
        .expect("query information_schema.tables");
    assert_eq!(
        table_count, 0,
        "bronze.events must not be built when its upstream loader step fails"
    );
}

/// Test: each of the four typed fan-out models
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`)
/// covers exactly the rows `silver.events_deduped` carries for its own
/// `type` — no row dropped, none duplicated by the `WHERE type =` filter.
#[test]
fn typed_fan_out_covers_every_event_of_its_type() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    for (relation, event_type) in [
        ("main.silver_push_events", "PushEvent"),
        ("main.silver_pr_events", "PullRequestEvent"),
        ("main.silver_issue_events", "IssuesEvent"),
        ("main.silver_star_events", "WatchEvent"),
    ] {
        let fan_out_rows = duckdb_scalar_i64(&db, &format!("SELECT count(*) FROM {relation}"));
        let deduped_rows = duckdb_scalar_i64(
            &db,
            &format!("SELECT count(*) FROM main.silver_events_deduped WHERE type = '{event_type}'"),
        );
        assert_eq!(
            fan_out_rows, deduped_rows,
            "{relation} must cover every {event_type} row in silver.events_deduped \
             ({fan_out_rows} vs {deduped_rows})"
        );
    }
}

/// Test: `silver.push_events`' extracted fields are (a) populated for every
/// row — the fixture measured every `PushEvent` payload key at 100% present
/// (`push_events.sql`'s own comment) — and (b) round-trip a direct
/// `JSON_EXTRACT_STRING` over the source `payload`, proving the
/// `JSON_EXTRACT_TEXT` registry emission on DuckDB actually extracts the
/// field it claims to, not just a non-NULL placeholder.
#[test]
fn push_events_extracts_typed_fields_from_payload() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, FIXTURE_DAYS);

    let missing = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM main.silver_push_events
         WHERE push_id IS NULL OR git_ref IS NULL OR head_sha IS NULL
            OR before_sha IS NULL",
    );
    assert_eq!(
        missing, 0,
        "every PushEvent payload field is present in the fixture"
    );

    let mismatched = duckdb_scalar_i64(
        &db,
        "SELECT count(*) FROM main.silver_push_events p
         JOIN main.silver_events_deduped d USING (id)
         WHERE p.git_ref != JSON_EXTRACT_STRING(d.payload, '$.ref')
            OR p.head_sha != JSON_EXTRACT_STRING(d.payload, '$.head')
            OR p.before_sha != JSON_EXTRACT_STRING(d.payload, '$.before')
            OR p.push_id != CAST(JSON_EXTRACT_STRING(d.payload, '$.push_id') AS BIGINT)",
    );
    assert_eq!(
        mismatched, 0,
        "push_events' extracted columns must match a direct JSON_EXTRACT_STRING \
         over the source payload"
    );
}
