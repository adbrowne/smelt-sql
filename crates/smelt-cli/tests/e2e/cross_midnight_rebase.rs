//! End-to-end test: the output window a `grain: partition` run writes must be
//! **derived** from the run window via the model's declared partition-column
//! skew, not pinned to the run window verbatim
//! (`docs/specs/model_transforms.md` §Semantics "The output window is
//! derived, never assumed").
//!
//! Fixture: a sessions-shaped model whose `partition_column`
//! (`session_start_date`) is a derived column that can skew away from the
//! driving `event_date` column, declared by the Form B filter `event_date
//! BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`
//! (the same shape as `examples/web_analytics` `silver/sessions.sql`, minus
//! the sessionization machinery itself — the source rows here carry a
//! precomputed `session_start_ts` directly, so the fixture isolates the
//! output-window derivation from the sessionize algorithm).
//!
//! Two events for the same device straddle midnight with a gap under the
//! sessionization's own cap (`session_start_ts` is the same for both, as a
//! real sessionize step would assign when the gap is small): one at
//! `2024-01-01 23:47`, one at `2024-01-02 00:03`. `2024-01-01`'s run
//! processes the source before the second event exists; `2024-01-02`'s run
//! processes it after — and its derived output window `[2024-01-01,
//! 2024-01-04)` reaches back into `2024-01-01`'s already-written partition
//! and rewrites it, extending the session across midnight.
//!
//! Before this landed, the DELETE range and output clamp were pinned to the
//! run window verbatim: `2024-01-02`'s run would never touch the
//! `2024-01-01` partition, leaving it permanently stale at `event_count = 1`.

use std::path::Path;
use std::sync::Arc;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, TimeseriesConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::{BatchedConfig, BatchedSafetyOverrides, Granularity, ModelDiscovery};
use smelt_logical::analysis::window_independence::{window_independence, WindowIndependence};
use smelt_maintenance_testkit::link_c_harness::SqlCapturingReporter;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::execute_project;
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::windowing::compute_incremental_windows;
use smelt_runtime::TimeRange;
use tokio_util::sync::CancellationToken;

struct DuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

/// Stage a project with:
/// - `models/sources/events.yml`: a `timeseries:`-declaring source
///   (partition_column: event_date) carrying a precomputed `session_start_ts`
///   column (standing in for a real sessionize step's output).
/// - `models/sessions.sql`: a `grain: partition` model whose own
///   `partition_column` (`session_start_date`) is derived from
///   `session_start_ts` and skews from `event_date` under the Form B filter.
fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = r#"description: Raw device events, timeseries by event_date, with a precomputed session_start_ts.
columns:
  - name: device_id
    type: INTEGER
  - name: event_ts
    type: TIMESTAMP
  - name: event_date
    type: DATE
  - name: session_start_ts
    type: TIMESTAMP
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
"#;
    std::fs::write(project_dir.join("models/sources/events.yml"), source_yml).unwrap();

    let model_sql = r#"---
materialization: table
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
refresh: incremental
grain: partition
---
WITH sessionized AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        session_start_ts,
        CAST(session_start_ts AS DATE) AS session_start_date
    FROM smelt.sources.events
)
SELECT
    device_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count
FROM sessionized
WHERE event_date
    BETWEEN session_start_date
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
"#;
    std::fs::write(project_dir.join("models/sessions.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: cross_midnight_rebase_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn seed_event(
    db_path: &Path,
    device_id: i64,
    event_ts: &str,
    event_date: &str,
    session_start_ts: &str,
    create_table: bool,
) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    if create_table {
        conn.execute_batch("CREATE SCHEMA IF NOT EXISTS main;")?;
        conn.execute_batch(&format!(
            "CREATE OR REPLACE TABLE main.sources_events AS \
             SELECT * FROM (VALUES \
                ({device_id}, TIMESTAMP '{event_ts}', DATE '{event_date}', TIMESTAMP '{session_start_ts}') \
             ) AS t(device_id, event_ts, event_date, session_start_ts);"
        ))?;
    } else {
        conn.execute_batch(&format!(
            "INSERT INTO main.sources_events VALUES \
             ({device_id}, TIMESTAMP '{event_ts}', DATE '{event_date}', TIMESTAMP '{session_start_ts}');"
        ))?;
    }
    Ok(())
}

/// Fetch `(event_count, session_end)` for the session row rooted at
/// `session_start_date`.
fn fetch_session(
    db_path: &Path,
    session_start_date: &str,
) -> anyhow::Result<Option<(i64, String)>> {
    let conn = duckdb::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT event_count, CAST(session_end AS VARCHAR) FROM main.sessions \
         WHERE session_start_date = ?::DATE",
    )?;
    let mut rows = stmt.query([session_start_date])?;
    if let Some(row) = rows.next()? {
        let count: i64 = row.get(0)?;
        let end: String = row.get(1)?;
        Ok(Some((count, end)))
    } else {
        Ok(None)
    }
}

async fn run_single_day_window(
    project_dir: &Path,
    db_path: &Path,
    config: &Arc<Config>,
    graph: &Arc<tokio::sync::Mutex<DependencyGraph>>,
    db: &Arc<tokio::sync::Mutex<smelt_db::Database>>,
    start: &str,
    end: &str,
) -> anyhow::Result<smelt_runtime::types::RunOutcome> {
    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
    };

    execute_project(
        format!("cross-midnight-{start}"),
        request,
        Arc::clone(config),
        Arc::clone(graph),
        Arc::clone(db),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
}

/// The day-46 shape (`docs/plans/20260710-web-analytics-maintenance-demo.md`)
/// in miniature: a session rooted at `23:47` on day D gains an event at
/// `00:03` on day D+1 (a 16-minute gap, well under the sessionization cap).
/// `D`'s single-day run only sees the first event; `D+1`'s single-day run —
/// processed after the second event lands — must derive an output window
/// reaching back into `D`'s already-written partition and rewrite it.
#[tokio::test]
async fn single_day_replay_rewrites_prior_day_partition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);

    // Only the first event exists when day 1 replays.
    seed_event(
        &db_path,
        1,
        "2024-01-01 23:47:00",
        "2024-01-01",
        "2024-01-01 23:47:00",
        true,
    )
    .expect("seed event 1");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let outcome = run_single_day_window(
        &project_dir,
        &db_path,
        &config,
        &graph,
        &db,
        "2024-01-01",
        "2024-01-02",
    )
    .await
    .expect("day 1 run must succeed");
    assert!(!outcome.models.is_empty(), "day 1: expected a model run");

    let day1_after_run1 =
        fetch_session(&db_path, "2024-01-01").expect("fetch session after day 1 run");
    assert_eq!(
        day1_after_run1,
        Some((1, "2024-01-01 23:47:00".to_string())),
        "day 1 alone must see event_count=1, session_end at 23:47"
    );

    // The second event lands (same session, 16-minute gap) before day 2 runs.
    seed_event(
        &db_path,
        1,
        "2024-01-02 00:03:00",
        "2024-01-02",
        "2024-01-01 23:47:00",
        false,
    )
    .expect("seed event 2");

    let outcome = run_single_day_window(
        &project_dir,
        &db_path,
        &config,
        &graph,
        &db,
        "2024-01-02",
        "2024-01-03",
    )
    .await
    .expect("day 2 run must succeed");
    assert!(!outcome.models.is_empty(), "day 2: expected a model run");

    let day1_after_run2 = fetch_session(&db_path, "2024-01-01")
        .expect("fetch session after day 2 run")
        .unwrap_or_else(|| {
            panic!(
                "day 2's derived output window must reach back and rewrite \
                 the 2024-01-01 partition, but no row was found there"
            )
        });
    assert_eq!(
        day1_after_run2,
        (2, "2024-01-02 00:03:00".to_string()),
        "day 2's run must rewrite the 2024-01-01 session to event_count=2, \
         session_end on 2024-01-02 — the derived output window \
         [2024-01-01, 2024-01-04) reaches back into the prior partition; \
         got {day1_after_run2:?}"
    );
}

// ── Two-boundary truncation: the declared relation is a semantic cap ────────

/// Batch-insert pre-sessionized events (`(event_ts, event_date,
/// session_start_ts)` rows) for device 1 into the staged source table.
fn seed_events_batch(db_path: &Path, rows: &[(String, String, String)]) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    let values: Vec<String> = rows
        .iter()
        .map(|(ts, date, root)| format!("(1, TIMESTAMP '{ts}', DATE '{date}', TIMESTAMP '{root}')"))
        .collect();
    conn.execute_batch(&format!(
        "INSERT INTO main.sources_events VALUES {};",
        values.join(", ")
    ))?;
    Ok(())
}

/// All session rows, ordered by `session_start`, as
/// `(session_start_date, event_count, session_start, session_end)`.
fn fetch_all_sessions(db_path: &Path) -> anyhow::Result<Vec<(String, i64, String, String)>> {
    let conn = duckdb::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT CAST(session_start_date AS VARCHAR), event_count, \
                CAST(session_start AS VARCHAR), CAST(session_end AS VARCHAR) \
         FROM main.sessions ORDER BY session_start",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The continuous two-boundary chain: 60 events for one device, every
/// pairwise gap under 30 minutes, spanning **two** midnights —
/// `2024-01-01 23:50`, then `2024-01-02 00:10` + 25-minute steps through
/// `2024-01-02 23:55`, then `2024-01-03 00:15`. Returns
/// `(event_ts, event_date, session_start_ts)` rows with `session_start_ts`
/// assigned exactly as `examples/web_analytics/functions/sessionize.sql`
/// computes it under the **clock-anchored cut**
/// (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
/// §"silver.sessions — clock-anchored cut"; this fixture's stand-in mirror —
/// the *real* function is pinned to the same shape end-to-end by
/// `per_partition_equivalence.rs::web_analytics_session_attribution_matches_full_rebuild`):
///
/// - The chain never breaks on inactivity or platform, so its only natural
///   boundary is the root, `2024-01-01 23:50`.
/// - The root's time-of-day (`23:50`) is `>= 00:30`, so its deadline is
///   `end_of_day(date(root) + 1 day)` = the midnight starting
///   `2024-01-03` — the *second* midnight. Every event strictly before that
///   deadline roots to `2024-01-01 23:50`: the root itself plus the full
///   25-minute grid through `2024-01-02 23:55` (58 events).
/// - The first event at or past the deadline (`2024-01-03 00:15`) is a
///   **forced root**: nothing marks it as a boundary for later events to
///   chain to, so it (and any further post-deadline event) roots its own
///   singleton session — it does not merge into the prior session, nor into
///   a second multi-event one.
fn two_boundary_chain() -> Vec<(String, String, String)> {
    let root = chrono::NaiveDateTime::parse_from_str("2024-01-01 23:50:00", "%Y-%m-%d %H:%M:%S")
        .expect("parse chain root");
    let day2_base =
        chrono::NaiveDateTime::parse_from_str("2024-01-02 00:10:00", "%Y-%m-%d %H:%M:%S")
            .expect("parse day-2 base");
    let tail = chrono::NaiveDateTime::parse_from_str("2024-01-03 00:15:00", "%Y-%m-%d %H:%M:%S")
        .expect("parse chain tail");

    let mut timestamps = vec![root];
    // k = 0..=57: 2024-01-02 00:10 … 2024-01-02 23:55 in 25-minute steps.
    for k in 0..=57 {
        timestamps.push(day2_base + chrono::Duration::minutes(25 * k));
    }
    timestamps.push(tail);

    // The root's time-of-day is >= 00:30, so its deadline reaches to the
    // *second* midnight: end_of_day(date(root) + 1 day).
    let deadline = (root.date() + chrono::Duration::days(2))
        .and_hms_opt(0, 0, 0)
        .expect("midnight");
    timestamps
        .into_iter()
        .map(|ts| {
            let assigned_root = if ts < deadline { root } else { ts };
            (
                ts.format("%Y-%m-%d %H:%M:%S").to_string(),
                ts.format("%Y-%m-%d").to_string(),
                assigned_root.format("%Y-%m-%d %H:%M:%S").to_string(),
            )
        })
        .collect()
}

/// The day-46 shape (above) stretched across **two** midnights
/// (`docs/specs/model_transforms.md` §Semantics "The output window is
/// derived, never assumed" — the "semantic cap" paragraph): a continuous
/// 60-event chain, every gap under 30 minutes, running `2024-01-01 23:50` →
/// `2024-01-03 00:15` — longer than the root's deadline. Only the
/// clock-anchored cut can end the session rooted at `23:50`; see
/// `two_boundary_chain` for the event grid and the mirrored sessionize root
/// assignment.
///
/// The root's time-of-day (`23:50`) is `>= 00:30`, so its deadline reaches
/// to the *second* midnight (the start of `2024-01-03`) — the model's Form B
/// relation (`event_date BETWEEN session_start_date AND session_start_date +
/// INTERVAL '1 day'`) declares exactly that one-day-forward reach in date
/// space, the declared relation the planner derives the output window from.
///
/// Claims pinned here:
/// - (a) truncation at the declared bound: the day-1 session carries
///   exactly the 59 in-deadline events (root plus the full day-2 grid;
///   `session_end = 2024-01-02 23:55`), never the post-deadline one — in a
///   day-by-day replay and a from-scratch full build alike;
/// - (b) excess-event behaviour: the first post-deadline event
///   (`2024-01-03 00:15`) roots its own **singleton** session — it is
///   neither absorbed into the day-1 session, nor dropped;
/// - (c) day-by-day ≡ full build on the full per-session tuple;
/// - (d) invariants: the device's sessions never overlap in time, and
///   event counts conserve (59 + 1 = 60 — no event lost or double-counted).
#[tokio::test]
async fn two_boundary_session_truncated_at_declared_bound() {
    let chain = two_boundary_chain();
    assert_eq!(chain.len(), 60, "fixture: expected a 60-event chain");
    let day1_rows: Vec<_> = chain
        .iter()
        .filter(|(_, d, _)| d == "2024-01-01")
        .cloned()
        .collect();
    let day2_rows: Vec<_> = chain
        .iter()
        .filter(|(_, d, _)| d == "2024-01-02")
        .cloned()
        .collect();
    let day3_rows: Vec<_> = chain
        .iter()
        .filter(|(_, d, _)| d == "2024-01-03")
        .cloned()
        .collect();
    assert_eq!(
        (day1_rows.len(), day2_rows.len(), day3_rows.len()),
        (1, 58, 1),
        "fixture: expected 1 + 58 + 1 events across the three days"
    );

    // ── Day-by-day replay: each day's events land before that day's run ──
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);

    let (ts, date, root) = &day1_rows[0];
    seed_event(&db_path, 1, ts, date, root, true).expect("seed day-1 event");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let outcome = run_single_day_window(
        &project_dir,
        &db_path,
        &config,
        &graph,
        &db,
        "2024-01-01",
        "2024-01-02",
    )
    .await
    .expect("day 1 run must succeed");
    assert!(!outcome.models.is_empty(), "day 1: expected a model run");
    assert_eq!(
        fetch_session(&db_path, "2024-01-01").expect("fetch after day 1"),
        Some((1, "2024-01-01 23:50:00".to_string())),
        "day 1 alone must see event_count=1"
    );

    seed_events_batch(&db_path, &day2_rows).expect("seed day-2 events");
    let outcome = run_single_day_window(
        &project_dir,
        &db_path,
        &config,
        &graph,
        &db,
        "2024-01-02",
        "2024-01-03",
    )
    .await
    .expect("day 2 run must succeed");
    assert!(!outcome.models.is_empty(), "day 2: expected a model run");

    assert_eq!(
        fetch_session(&db_path, "2024-01-01").expect("fetch day-1 partition after day 2"),
        Some((59, "2024-01-02 23:55:00".to_string())),
        "(a) day 2's derived output window [2024-01-01, 2024-01-04) must \
         rewrite the day-1 session with all 59 in-deadline events — the \
         root plus the full day-2 grid — so session_end is the chain's \
         last day-2 event (2024-01-02 23:55)"
    );
    assert_eq!(
        fetch_session(&db_path, "2024-01-02").expect("fetch day-2 partition after day 2"),
        None,
        "no session should root in the 2024-01-02 partition yet — every \
         day-2 event is still within the day-1 root's deadline"
    );

    seed_events_batch(&db_path, &day3_rows).expect("seed day-3 event");
    let outcome = run_single_day_window(
        &project_dir,
        &db_path,
        &config,
        &graph,
        &db,
        "2024-01-03",
        "2024-01-04",
    )
    .await
    .expect("day 3 run must succeed");
    assert!(!outcome.models.is_empty(), "day 3: expected a model run");

    let day_by_day = fetch_all_sessions(&db_path).expect("fetch all sessions (day-by-day)");
    assert_eq!(
        day_by_day,
        vec![
            (
                "2024-01-01".to_string(),
                59,
                "2024-01-01 23:50:00".to_string(),
                "2024-01-02 23:55:00".to_string(),
            ),
            (
                "2024-01-03".to_string(),
                1,
                "2024-01-03 00:15:00".to_string(),
                "2024-01-03 00:15:00".to_string(),
            ),
        ],
        "(a)+(b) final day-by-day state: the 59-event day-1 session (root \
         plus the full day-2 grid) plus one singleton post-deadline session \
         — the day-1 session must survive day 3's run unchanged"
    );

    // (d) invariants: non-overlap + event conservation.
    for pair in day_by_day.windows(2) {
        let (_, _, _, prev_end) = &pair[0];
        let (_, _, next_start, _) = &pair[1];
        assert!(
            next_start > prev_end,
            "(d) sessions must not overlap in time: session starting \
             {next_start} begins at or before the previous session's end \
             {prev_end}"
        );
    }
    let total: i64 = day_by_day.iter().map(|(_, n, _, _)| n).sum();
    assert_eq!(
        total, 60,
        "(d) event conservation: the device's sessions must account for \
         every injected event exactly once (59 + 1 = 60)"
    );

    // ── From-scratch full-window build over the same 60 events ──
    let tmp2 = tempfile::TempDir::new().expect("tempdir");
    let project_dir2 = tmp2.path().to_path_buf();
    let db_path2 = project_dir2.join("dev.duckdb");

    stage_project(&project_dir2, &db_path2);
    let (ts, date, root) = &chain[0];
    seed_event(&db_path2, 1, ts, date, root, true).expect("full build: seed first event");
    seed_events_batch(&db_path2, &chain[1..]).expect("full build: seed remaining events");

    let config2 = Arc::new(Config::load(&project_dir2).expect("load config"));
    let (db2, graph2) = build_db_and_graph(&project_dir2, &config2);

    let outcome = run_single_day_window(
        &project_dir2,
        &db_path2,
        &config2,
        &graph2,
        &db2,
        "2024-01-01",
        "2024-01-04",
    )
    .await
    .expect("full-window run must succeed");
    assert!(
        !outcome.models.is_empty(),
        "full-window build: expected a model run"
    );

    let full = fetch_all_sessions(&db_path2).expect("fetch all sessions (full build)");
    assert_eq!(
        full, day_by_day,
        "(c) the from-scratch full-window build must equal the day-by-day \
         replay row-for-row — truncation at the declared bound is a property \
         of the model's own SQL, identical in every build shape"
    );
}

// ── Never-idle device: the clock-anchored cut eliminates confetti ──────────

/// The clock-anchored rule's deadline for a candidate root
/// (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
/// §"silver.sessions — clock-anchored cut"): `end_of_day(date(root))` if the
/// root's time-of-day is before 00:30, else `end_of_day(date(root) + 1
/// day)`.
fn deadline_for(root: NaiveDateTime) -> NaiveDateTime {
    let midnight = NaiveTime::from_hms_opt(0, 0, 0).expect("midnight");
    let cutoff = NaiveTime::from_hms_opt(0, 30, 0).expect("00:30");
    let days_ahead = if root.time() < cutoff { 1 } else { 2 };
    NaiveDateTime::new(root.date() + Duration::days(days_ahead), midnight)
}

/// Mirrors `examples/web_analytics/functions/sessionize.sql`'s clock-anchored
/// root assignment for a single-partition, single-platform, continuous
/// chain: natural boundaries are gap-only (no platform change here), the
/// candidate is the most recent natural boundary within a trailing 2-day
/// window, and an event past its candidate's deadline is a forced root
/// (assigned to the first event of its own calendar day).
fn assign_session_roots(events: &[NaiveDateTime]) -> Vec<NaiveDateTime> {
    let mut natural_boundaries: Vec<NaiveDateTime> = Vec::new();
    for (i, &ts) in events.iter().enumerate() {
        let is_boundary = match events.get(i.wrapping_sub(1)) {
            Some(&prev) if i > 0 => ts - prev > Duration::minutes(30),
            _ => true,
        };
        if is_boundary {
            natural_boundaries.push(ts);
        }
    }

    events
        .iter()
        .map(|&ts| {
            let candidate = natural_boundaries
                .iter()
                .filter(|&&nts| nts <= ts && ts - nts <= Duration::days(2))
                .max()
                .copied();
            match candidate {
                Some(c) if ts < deadline_for(c) => c,
                _ => events
                    .iter()
                    .filter(|&&e| e.date() == ts.date())
                    .min()
                    .copied()
                    .expect("own day has at least this event"),
            }
        })
        .collect()
}

/// A never-idle device (one event every 29 minutes, gap always under the
/// 30-minute inactivity threshold, so the chain never strikes a natural
/// boundary after its root) rooted at `2024-01-01 14:00`, running for 9
/// elapsed days. Under the old frame-cap + `COALESCE` fallback design this
/// degenerated into single-event confetti after the first capped session
/// (`docs/research/20260711-clock-vs-root-anchored-sessions.md` §Problem);
/// under the clock-anchored cut it settles into a deterministic cadence: one
/// ~34-hour session (the root's own 2-day reach, since `14:00 >= 00:30`),
/// then exactly one session per subsequent calendar day, phase-locked to
/// each day's first event.
#[tokio::test]
async fn never_idle_device_yields_one_session_per_day() {
    let root = NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("date")
        .and_hms_opt(14, 0, 0)
        .expect("time");
    let end = root + Duration::days(9);

    let mut timestamps = Vec::new();
    let mut t = root;
    while t < end {
        timestamps.push(t);
        t += Duration::minutes(29);
    }
    assert_eq!(timestamps.len(), 447, "fixture: expected 447 events");

    let roots = assign_session_roots(&timestamps);
    let chain: Vec<(String, String, String)> = timestamps
        .iter()
        .zip(&roots)
        .map(|(ts, root)| {
            (
                ts.format("%Y-%m-%d %H:%M:%S").to_string(),
                ts.format("%Y-%m-%d").to_string(),
                root.format("%Y-%m-%d %H:%M:%S").to_string(),
            )
        })
        .collect();

    // Group rows by calendar day, in order, for day-by-day replay.
    type ChainRow = (String, String, String);
    let mut days: Vec<(String, Vec<ChainRow>)> = Vec::new();
    for row in &chain {
        match days.last_mut() {
            Some((d, rows)) if *d == row.1 => rows.push(row.clone()),
            _ => days.push((row.1.clone(), vec![row.clone()])),
        }
    }
    assert_eq!(
        days.len(),
        10,
        "fixture: expected events across 10 calendar days"
    );

    // ── Day-by-day replay ──
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");
    stage_project(&project_dir, &db_path);

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let mut first = true;
    for (day, rows) in &days {
        if first {
            let (ts, date, root) = &rows[0];
            seed_event(&db_path, 1, ts, date, root, true).expect("seed first event");
            if rows.len() > 1 {
                seed_events_batch(&db_path, &rows[1..]).expect("seed remaining first-day events");
            }
            first = false;
        } else {
            seed_events_batch(&db_path, rows).expect("seed day's events");
        }

        let start = day.clone();
        let end_date =
            NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("parse day") + Duration::days(1);
        let outcome = run_single_day_window(
            &project_dir,
            &db_path,
            &config,
            &graph,
            &db,
            &start,
            &end_date.format("%Y-%m-%d").to_string(),
        )
        .await
        .unwrap_or_else(|e| panic!("run for {day} must succeed: {e}"));
        assert!(!outcome.models.is_empty(), "{day}: expected a model run");
    }

    let day_by_day = fetch_all_sessions(&db_path).expect("fetch all sessions (day-by-day)");
    assert_eq!(
        day_by_day.len(),
        9,
        "expected 9 sessions (one ~34h root session, then one per \
         subsequent day), got {day_by_day:?}"
    );
    assert!(
        day_by_day.iter().all(|(_, n, _, _)| *n > 1),
        "no session may degenerate to a single-event confetti session, got \
         {day_by_day:?}"
    );
    let total: i64 = day_by_day.iter().map(|(_, n, _, _)| n).sum();
    assert_eq!(
        total, 447,
        "every event must be accounted for exactly once across sessions"
    );
    for pair in day_by_day.windows(2) {
        let (_, _, _, prev_end) = &pair[0];
        let (_, _, next_start, _) = &pair[1];
        assert!(
            next_start > prev_end,
            "sessions must not overlap: session starting {next_start} \
             begins at or before the previous session's end {prev_end}"
        );
    }

    // ── From-scratch full-window build over the same 447 events ──
    let tmp2 = tempfile::TempDir::new().expect("tempdir");
    let project_dir2 = tmp2.path().to_path_buf();
    let db_path2 = project_dir2.join("dev.duckdb");
    stage_project(&project_dir2, &db_path2);

    let (ts, date, root) = &chain[0];
    seed_event(&db_path2, 1, ts, date, root, true).expect("full build: seed first event");
    seed_events_batch(&db_path2, &chain[1..]).expect("full build: seed remaining events");

    let config2 = Arc::new(Config::load(&project_dir2).expect("load config"));
    let (db2, graph2) = build_db_and_graph(&project_dir2, &config2);

    let outcome = run_single_day_window(
        &project_dir2,
        &db_path2,
        &config2,
        &graph2,
        &db2,
        "2024-01-01",
        "2024-01-11",
    )
    .await
    .expect("full-window run must succeed");
    assert!(
        !outcome.models.is_empty(),
        "full-window build: expected a model run"
    );

    let full = fetch_all_sessions(&db_path2).expect("fetch all sessions (full build)");
    assert_eq!(
        full, day_by_day,
        "the from-scratch full-window build must equal the day-by-day \
         replay row-for-row — window-independence holds under a never-idle \
         input"
    );
}

// ── Identity model: derived output window equals the run window ──────────────

fn make_ts(event_col: &str, partition_col: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn make_inc() -> BatchedConfig {
    BatchedConfig {
        unique_key: vec![],
        nondeterministic_columns: vec![],
        safety_overrides: BatchedSafetyOverrides::default(),
    }
}

/// A zero-skew (identity) model — `partition_column` tracks the event-time
/// column directly, with no Form B relation anchored on it — must derive an
/// output window equal to the run window verbatim, exactly as before this
/// feature existed (`docs/specs/model_transforms.md` §Semantics "Identity
/// (the common case)").
#[test]
fn identity_model_windows_unchanged() {
    let sql = "SELECT event_date, COUNT(*) AS n FROM smelt.silver.events GROUP BY event_date";
    let ts = make_ts("event_date", "event_date");
    let inc = make_inc();
    let range = TimeRange {
        start: "2026-04-10".to_string(),
        end: "2026-04-12".to_string(),
    };

    let windows =
        compute_incremental_windows(&ts, &inc, sql, &Default::default(), 0, &range, None, false)
            .expect("identity model must not be refused");

    assert_eq!(windows.batches.len(), 1, "expected a single batch");
    let b = &windows.batches[0];
    assert_eq!(
        b.partition_start,
        NaiveDate::parse_from_str("2026-04-10", "%Y-%m-%d").unwrap(),
        "identity model's DELETE/clamp range must start at the run window verbatim"
    );
    assert_eq!(
        b.partition_end,
        NaiveDate::parse_from_str("2026-04-12", "%Y-%m-%d").unwrap(),
        "identity model's DELETE/clamp range must end at the run window verbatim"
    );
}

// ── Chained (root-anchored, self-referential) table ────────────────────────
//
// `docs/research/20260711-clock-vs-root-anchored-sessions.md`
// §"silver.sessions_chained — root-anchored cut". Unlike the clock-anchored
// fixtures above (a hand-rolled analogue of `sessionize`'s own closed form),
// these tests exercise the REAL, shipped
// `examples/web_analytics/models/silver/sessions_chained.sql` model text
// directly — self-referential ordered execution is exactly the seam this
// plan's Phase 1 (`compute_incremental_windows_ordered` composing with the
// derived output window) exists to prove sound, and a hand-copied analogue
// would risk silently drifting from what actually ships.

/// The real model file, read byte-for-byte — never a hand-copied analogue.
/// Its own external dependency address (`smelt.silver.events_parsed`) is
/// reused unchanged by staging a plain source at that same path, so no text
/// substitution is needed.
fn chained_model_sql() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("repo root")
        .join("examples/web_analytics/models/silver/sessions_chained.sql");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read sessions_chained.sql from {path:?}: {e}"))
}

/// Stage a project with:
/// - `models/silver/events_parsed.yml`: a plain source carrying exactly the
///   columns `silver.sessions_chained` reads (`device_id`, `event_ts`,
///   `event_date`, `platform`, `utm_campaign`).
/// - `models/silver/sessions_chained.sql`: the real model file, unmodified.
fn stage_chained_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/silver")).unwrap();

    let source_yml = r#"description: Raw device events (root-anchored chained fixture).
columns:
  - name: device_id
    type: INTEGER
  - name: event_ts
    type: TIMESTAMP
  - name: event_date
    type: DATE
  - name: platform
    type: VARCHAR
  - name: utm_campaign
    type: VARCHAR
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
"#;
    std::fs::write(
        project_dir.join("models/silver/events_parsed.yml"),
        source_yml,
    )
    .unwrap();

    std::fs::write(
        project_dir.join("models/silver/sessions_chained.sql"),
        chained_model_sql(),
    )
    .unwrap();

    let smelt_yml = format!(
        "name: chained_rebase_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Create the (empty) source events table.
fn create_chained_events_table(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.silver_events_parsed ( \
             device_id INTEGER, event_ts TIMESTAMP, event_date DATE, \
             platform VARCHAR, utm_campaign VARCHAR \
         );",
    )?;
    Ok(())
}

/// Bootstrap the self-referential table with a placeholder row dated well
/// outside any run window these tests use — DuckDB refuses `CREATE TABLE ...
/// AS SELECT` self-referencing itself for a model's very first partition
/// (the same sidestep `crates/smelt-runtime/tests/self_referential_ordered_backfill.rs`,
/// `crates/smelt-cli/tests/e2e/two_layer_clamp.rs`, and
/// `crates/smelt-cli/tests/property_discovery/g_13_self_ref_derived_output_window.rs`
/// use).
fn bootstrap_chained_table(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE TABLE main.silver_sessions_chained ( \
             session_id VARCHAR, device_id INTEGER, session_start_ts TIMESTAMP, \
             session_start_date DATE, session_start TIMESTAMP, session_end TIMESTAMP, \
             event_count BIGINT, platform VARCHAR, utm_campaign VARCHAR \
         ); \
         INSERT INTO main.silver_sessions_chained VALUES ( \
             'bootstrap', 0, TIMESTAMP '1970-01-01 00:00:00', DATE '1970-01-01', \
             TIMESTAMP '1970-01-01 00:00:00', TIMESTAMP '1970-01-01 00:00:00', 0, 'bootstrap', NULL \
         );",
    )?;
    Ok(())
}

fn seed_chained_events_batch(db_path: &Path, rows: &[(String, String)]) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    let values: Vec<String> = rows
        .iter()
        .map(|(ts, date)| format!("(1, TIMESTAMP '{ts}', DATE '{date}', 'web', NULL)"))
        .collect();
    conn.execute_batch(&format!(
        "INSERT INTO main.silver_events_parsed VALUES {};",
        values.join(", ")
    ))?;
    Ok(())
}

/// All chained session rows for device 1 (the bootstrap placeholder is
/// device 0, filtered out), ordered by `session_start`, as
/// `(session_start_date, event_count, session_start, session_end)`.
fn fetch_all_chained_sessions(
    db_path: &Path,
) -> anyhow::Result<Vec<(String, i64, String, String)>> {
    let conn = duckdb::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT CAST(session_start_date AS VARCHAR), event_count, \
                CAST(session_start AS VARCHAR), CAST(session_end AS VARCHAR) \
         FROM main.silver_sessions_chained \
         WHERE device_id = 1 \
         ORDER BY session_start",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mirrors `sessions_chained.sql`'s own closed form (no clock alignment): a
/// natural (gap > 30 min) boundary within a trailing 2-day window is a local
/// chain's candidate root; that root is assigned as long as
/// `floor(days_between(root_date, event_date) / 2) == 0` (the root-anchored
/// 2-day reach, day-granularity — the model's own Form B relation, `event_date
/// BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`);
/// once exceeded, the FIRST event in the next 2-day epoch becomes the new
/// root for that epoch, and so on (the non-clock analogue of `sessionize`'s
/// own day-aligned forced-root trick — see the model's `_bucketed` CTE).
fn assign_session_roots_chained(events: &[NaiveDateTime]) -> Vec<NaiveDateTime> {
    let mut natural_boundaries: Vec<NaiveDateTime> = Vec::new();
    for (i, &ts) in events.iter().enumerate() {
        let is_boundary = match events.get(i.wrapping_sub(1)) {
            Some(&prev) if i > 0 => ts - prev > Duration::minutes(30),
            _ => true,
        };
        if is_boundary {
            natural_boundaries.push(ts);
        }
    }
    // Unlike the model's own `_local_root_ts` (a 2-day trailing window
    // FRAME, bounded by what a single real invocation's own read window
    // actually contains), this oracle computes the emergent outcome across
    // the WHOLE logical history (many separate day-by-day invocations, each
    // continuing via the self-reference's stored state, not a window frame)
    // — so the most recent natural boundary is always the candidate here,
    // regardless of elapsed distance; the root-anchored 2-day reach is
    // enforced entirely by the epoch-bucketing below.
    let local_root_for = |ts: NaiveDateTime| -> NaiveDateTime {
        natural_boundaries
            .iter()
            .filter(|&&b| b <= ts)
            .max()
            .copied()
            .expect("own day has at least this event")
    };

    events
        .iter()
        .map(|&ts| {
            let local_root = local_root_for(ts);
            let anchor_date = local_root.date();
            let bucket = (ts.date() - anchor_date).num_days() / 2;
            if bucket == 0 {
                local_root
            } else {
                events
                    .iter()
                    .filter(|&&e| {
                        local_root_for(e) == local_root
                            && (e.date() - anchor_date).num_days() / 2 == bucket
                    })
                    .min()
                    .copied()
                    .expect("bucket contains at least this event")
            }
        })
        .collect()
}

/// The same never-idle device as `never_idle_device_yields_one_session_per_day`
/// (one event every 29 minutes for 9 elapsed days), fed through the
/// root-anchored `silver.sessions_chained` model instead of the clock-anchored
/// `silver.sessions`: no single-event confetti, every event counted exactly
/// once, and — because the cutoff's phase is inherited from each session's
/// own root rather than the clock — roughly one session per TWO days
/// (half the clock table's own ~1-per-day cadence), phase-locked to the
/// original 14:00 root's own history. In-order day-by-day replay (the
/// `Ordered` self-edge's own requirement) must equal a from-scratch
/// sequential build.
#[tokio::test]
async fn chained_never_idle_device_yields_one_session_per_two_days() {
    let root = NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("date")
        .and_hms_opt(14, 0, 0)
        .expect("time");
    let end = root + Duration::days(9);

    let mut timestamps = Vec::new();
    let mut t = root;
    while t < end {
        timestamps.push(t);
        t += Duration::minutes(29);
    }
    assert_eq!(timestamps.len(), 447, "fixture: expected 447 events");

    let roots = assign_session_roots_chained(&timestamps);
    let mut unique_roots: Vec<NaiveDateTime> = roots.clone();
    unique_roots.dedup();
    assert_eq!(
        unique_roots.len(),
        5,
        "expected exactly 5 sessions (~1 per 2 days over 9 elapsed days, \
         half the clock table's own ~1-per-day cadence — the deterministic \
         closed-form outcome for this fixture, quoted verbatim by the \
         generated tutorial's never-idle comparison table), got {:?}",
        unique_roots
    );

    // Expected (session_start_date, event_count, session_start, session_end)
    // rows, derived purely from the model's own closed form — never a magic
    // hand-picked number.
    let mut groups: Vec<(NaiveDateTime, Vec<NaiveDateTime>)> = Vec::new();
    for (&ts, &root) in timestamps.iter().zip(&roots) {
        match groups.last_mut() {
            Some((r, members)) if *r == root => members.push(ts),
            _ => groups.push((root, vec![ts])),
        }
    }
    let expected: Vec<(String, i64, String, String)> = groups
        .iter()
        .map(|(root, members)| {
            (
                root.format("%Y-%m-%d").to_string(),
                members.len() as i64,
                members
                    .first()
                    .expect("group is non-empty")
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string(),
                members
                    .last()
                    .expect("group is non-empty")
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string(),
            )
        })
        .collect();

    let chain: Vec<(String, String)> = timestamps
        .iter()
        .map(|ts| {
            (
                ts.format("%Y-%m-%d %H:%M:%S").to_string(),
                ts.format("%Y-%m-%d").to_string(),
            )
        })
        .collect();

    // Group by calendar day, in order, for the (Ordered) day-by-day replay —
    // the self-edge's own requirement is that these run strictly sequentially.
    type ChainRow = (String, String);
    let mut days: Vec<(String, Vec<ChainRow>)> = Vec::new();
    for row in &chain {
        match days.last_mut() {
            Some((d, rows)) if *d == row.1 => rows.push(row.clone()),
            _ => days.push((row.1.clone(), vec![row.clone()])),
        }
    }

    // ── Day-by-day (Ordered, sequential) replay ──
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");
    stage_chained_project(&project_dir, &db_path);
    create_chained_events_table(&db_path).expect("create events table");
    bootstrap_chained_table(&db_path).expect("bootstrap chained table");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    for (day, rows) in &days {
        seed_chained_events_batch(&db_path, rows).expect("seed day's events");

        let start = day.clone();
        let end_date =
            NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("parse day") + Duration::days(1);
        let outcome = run_single_day_window(
            &project_dir,
            &db_path,
            &config,
            &graph,
            &db,
            &start,
            &end_date.format("%Y-%m-%d").to_string(),
        )
        .await
        .unwrap_or_else(|e| panic!("run for {day} must succeed: {e}"));
        assert!(!outcome.models.is_empty(), "{day}: expected a model run");
    }

    let day_by_day = fetch_all_chained_sessions(&db_path).expect("fetch all sessions (day-by-day)");
    assert_eq!(
        day_by_day, expected,
        "day-by-day replay must match the model's own closed form exactly"
    );
    assert!(
        day_by_day.iter().all(|(_, n, _, _)| *n > 1),
        "no session may degenerate to a single-event confetti session, got \
         {day_by_day:?}"
    );
    let total: i64 = day_by_day.iter().map(|(_, n, _, _)| n).sum();
    assert_eq!(
        total, 447,
        "every event must be accounted for exactly once across sessions"
    );
    for pair in day_by_day.windows(2) {
        let (_, _, _, prev_end) = &pair[0];
        let (_, _, next_start, _) = &pair[1];
        assert!(
            next_start > prev_end,
            "sessions must not overlap: session starting {next_start} \
             begins at or before the previous session's end {prev_end}"
        );
    }

    // ── From-scratch sequential build over the same 447 events, requested
    // as a single multi-day window — the Ordered self-edge still forces
    // strictly sequential single-partition batches internally, so this must
    // converge to the identical trajectory as the day-by-day replay above.
    let tmp2 = tempfile::TempDir::new().expect("tempdir");
    let project_dir2 = tmp2.path().to_path_buf();
    let db_path2 = project_dir2.join("dev.duckdb");
    stage_chained_project(&project_dir2, &db_path2);
    create_chained_events_table(&db_path2).expect("create events table");
    bootstrap_chained_table(&db_path2).expect("bootstrap chained table");
    seed_chained_events_batch(&db_path2, &chain).expect("full build: seed all events");

    let config2 = Arc::new(Config::load(&project_dir2).expect("load config"));
    let (db2, graph2) = build_db_and_graph(&project_dir2, &config2);

    let outcome = run_single_day_window(
        &project_dir2,
        &db_path2,
        &config2,
        &graph2,
        &db2,
        "2024-01-01",
        "2024-01-11",
    )
    .await
    .expect("full-window run must succeed");
    assert!(
        !outcome.models.is_empty(),
        "full-window build: expected a model run"
    );

    let full = fetch_all_chained_sessions(&db_path2).expect("fetch all sessions (full build)");
    assert_eq!(
        full, day_by_day,
        "a single multi-day request must converge to the identical \
         trajectory as the day-by-day replay — the Ordered self-edge forces \
         the same strictly-sequential batch order internally either way"
    );
}

/// The planner verdict for `silver.sessions_chained` must be `Ordered` (via
/// the real proof — `window_independence`'s own backward-bound derivation
/// over the model's actual SQL — never a declaration or override), and a
/// multi-day run must execute as sequential single-partition batches, never
/// parallel and never a single wide batch.
#[tokio::test]
async fn chained_run_is_refused_or_ordered_never_parallel() {
    // (1) Planner verdict, proved directly over the shipped model text.
    let model_sql = chained_model_sql();
    let clean = smelt_parser::strip_frontmatter(&model_sql);
    let refs = vec![
        "silver.events_parsed".to_string(),
        "silver.sessions_chained".to_string(),
    ];
    let verdict = window_independence(
        "silver.sessions_chained",
        &refs,
        Some("session_start_date"),
        &clean,
    );
    assert_eq!(
        verdict,
        WindowIndependence::Ordered,
        "the real sessions_chained.sql model must prove Ordered via its \
         backward-bounded (no forward reach) self-read, got {verdict:?} \
         instead — never WindowIndependent (it has a genuine self-edge) and \
         never Refused (the self-edge's own bound is backward-only)"
    );

    // (2) A multi-day run must compile and execute as sequential
    // single-partition batches. Well-separated (>30 min gap) single events
    // per day keep each day trivially its own session, isolating the
    // batch-ordering assertion from the sessionization logic itself.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");
    stage_chained_project(&project_dir, &db_path);
    create_chained_events_table(&db_path).expect("create events table");
    bootstrap_chained_table(&db_path).expect("bootstrap chained table");
    seed_chained_events_batch(
        &db_path,
        &[
            ("2024-02-01 10:00:00".to_string(), "2024-02-01".to_string()),
            ("2024-02-02 10:00:00".to_string(), "2024-02-02".to_string()),
            ("2024-02-03 10:00:00".to_string(), "2024-02-03".to_string()),
        ],
    )
    .expect("seed 3 days of well-separated events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-02-01".to_string()),
        end: Some("2024-02-04".to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
    };

    let reporter = SqlCapturingReporter::new();
    execute_project(
        "chained-ordered-check".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("multi-day chained run must succeed");

    // The requested [2024-02-01, 2024-02-04) window rebases one day earlier
    // (Phase 1's ordered × derived-output-window composition: the model's
    // own Form B relation declares a 1-day skew) to [2024-01-31, 2024-02-04)
    // — 4 sequential single-partition batches, not 3 — while still building
    // strictly in temporal order (Ordered's own forcing, unaffected by the
    // rebase).
    let compiled = reporter.sql_for("silver.sessions_chained");
    assert_eq!(
        compiled.len(),
        4,
        "the rebased [2024-01-31, 2024-02-04) window must execute as 4 \
         sequential single-partition batches, got: {compiled:?}"
    );
    let expected_output_clamps = [
        "session_start_date >= '2024-01-31' AND session_start_date < '2024-02-01'",
        "session_start_date >= '2024-02-01' AND session_start_date < '2024-02-02'",
        "session_start_date >= '2024-02-02' AND session_start_date < '2024-02-03'",
        "session_start_date >= '2024-02-03' AND session_start_date < '2024-02-04'",
    ];
    for (i, expected_clamp) in expected_output_clamps.iter().enumerate() {
        assert!(
            compiled[i].contains(expected_clamp),
            "batch {} must be built with output clamp `{}`, built strictly \
             after the previous batch: {}",
            i + 1,
            expected_clamp,
            compiled[i]
        );
    }
}
