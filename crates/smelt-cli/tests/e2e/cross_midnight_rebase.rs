//! End-to-end test: the output window a `grain: partition` run writes must be
//! **derived** from the run window via the model's declared partition-column
//! skew, not pinned to the run window verbatim
//! (`docs/specs/model_transforms.md` §Semantics "The output window is
//! derived, never assumed").
//!
//! Fixture: a sessions-shaped model whose `partition_column`
//! (`session_start_date`) is a derived column that can skew away from the
//! driving `event_date` column, declared by the Form B filter `event_date
//! BETWEEN session_start_date - INTERVAL '1 day' AND session_start_date +
//! INTERVAL '1 day'` (the same shape as `examples/web_analytics`
//! `silver/sessions.sql`, minus the sessionization machinery itself — the
//! source rows here carry a precomputed `session_start_ts` directly, so the
//! fixture isolates the output-window derivation from the sessionize
//! algorithm).
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

use chrono::NaiveDate;
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, TimeseriesConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::{BatchedConfig, BatchedSafetyOverrides, Granularity, ModelDiscovery};
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
    BETWEEN session_start_date - INTERVAL '1 day'
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
/// computes it (this fixture's stand-in mirror; the *real* function is
/// pinned to the same shape end-to-end by
/// `per_partition_equivalence.rs::web_analytics_session_attribution_matches_full_rebuild`):
///
/// - The chain never breaks on inactivity or platform, so its only session
///   boundary (`_boundary_ts`) is the root, `2024-01-01 23:50`.
/// - An event roots to that boundary iff the boundary is inside its
///   `RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW` frame —
///   i.e. iff `event_ts <= root + 1 day` (**timestamp** granularity). That
///   covers the root plus the 25-minute grid through `2024-01-02 23:30`
///   (58 events).
/// - An event past `root + 1 day` (`2024-01-02 23:55` and
///   `2024-01-03 00:15`) sees no boundary in frame, and
///   `COALESCE(MAX(_boundary_ts) OVER …, ts_col)` falls back to its **own**
///   timestamp. The fallback does not strike a new `_boundary_ts` later
///   events could chain to, so each post-cap event is its own singleton
///   session — they do *not* merge into a second multi-event session.
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

    let cap = root + chrono::Duration::days(1);
    timestamps
        .into_iter()
        .map(|ts| {
            let assigned_root = if ts <= cap { root } else { ts };
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
/// `2024-01-03 00:15` — longer than the declared 1-day cap. Only the cap
/// can end the session rooted at `23:50`; see `two_boundary_chain` for the
/// event grid and the mirrored sessionize root assignment.
///
/// Note the cap's two granularities: the *operative* row-membership cut is
/// sessionize's frame reach in **timestamp** space (`event_ts <= root + 1
/// day`, so the `2024-01-02 23:55` event is out despite its `event_date`
/// being within ±1 day of the root's date), while the model's Form B
/// relation restates the same 1-day cap in **date** space over
/// `(event_date, session_start_date)` — the declared relation the planner
/// derives the output window from. The date-space filter is implied by the
/// timestamp-space one (it removes no rows the sessionize cap kept), which
/// is exactly why declaring it is sound.
///
/// Claims pinned here:
/// - (a) truncation at the declared bound: the day-1 session carries
///   exactly the 58 in-cap events (`session_end = 2024-01-02 23:30`), never
///   the post-cap ones — in a day-by-day replay and a from-scratch full
///   build alike;
/// - (b) excess-event behaviour: each post-cap event roots its own
///   **singleton** session (`2024-01-02 23:55`, `2024-01-03 00:15`) — they
///   are neither absorbed, nor dropped, nor merged together;
/// - (c) day-by-day ≡ full build on the full per-session tuple;
/// - (d) invariants: the device's sessions never overlap in time, and
///   event counts conserve (58 + 1 + 1 = 60 — no event lost or
///   double-counted).
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
        Some((58, "2024-01-02 23:30:00".to_string())),
        "(a) day 2's derived output window [2024-01-01, 2024-01-04) must \
         rewrite the day-1 session with exactly the 58 in-cap events — \
         truncated at root + 1 day, so session_end is the last in-cap event \
         (2024-01-02 23:30), not the chain's last day-2 event (23:55)"
    );
    assert_eq!(
        fetch_session(&db_path, "2024-01-02").expect("fetch day-2 partition after day 2"),
        Some((1, "2024-01-02 23:55:00".to_string())),
        "(b) the first post-cap event (2024-01-02 23:55) must root its own \
         singleton session in the 2024-01-02 partition"
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
                58,
                "2024-01-01 23:50:00".to_string(),
                "2024-01-02 23:30:00".to_string(),
            ),
            (
                "2024-01-02".to_string(),
                1,
                "2024-01-02 23:55:00".to_string(),
                "2024-01-02 23:55:00".to_string(),
            ),
            (
                "2024-01-03".to_string(),
                1,
                "2024-01-03 00:15:00".to_string(),
                "2024-01-03 00:15:00".to_string(),
            ),
        ],
        "(a)+(b) final day-by-day state: the truncated 58-event day-1 \
         session plus two singleton post-cap sessions — the post-cap events \
         must not merge into one session (falling out of the frame strikes \
         no new boundary), and the day-1 session must survive day 3's run \
         unchanged"
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
         every injected event exactly once (58 + 1 + 1 = 60)"
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
