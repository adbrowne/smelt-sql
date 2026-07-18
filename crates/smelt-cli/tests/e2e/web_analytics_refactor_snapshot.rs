#![cfg(feature = "duckdb")]
//! Snapshot tests for `examples/web_analytics/` refactors.
//!
//! Phase 1.3: verifies that refactoring `events_parsed.sql` to use
//! `smelt.functions.parse_event_payload(payload).*` produces byte-equal rows
//! to the original inlined `json_extract_string(...)` calls.
//!
//! Phase 1.4: verifies that refactoring `sessions.sql` to call
//! `smelt.functions.sessionize(source => ..., partition_col => ...)` with
//! named arguments produces an identical row set to the original three-CTE
//! inlined pipeline.
//!
//! Both tests stage minimal self-contained workspaces (no Parquet generation
//! from smelt-datagen) and use deterministic CSV seeds.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ─── Helpers (duplicated from functions_e2e.rs to keep tests self-contained) ──

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_workspace(tmp: &Path, files: &[(&str, &str)]) {
    for (rel, contents) in files {
        let path = tmp.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write workspace file");
    }
}

fn run_smelt_build(project_dir: &Path, target: &str) {
    let output = Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    assert!(
        output.status.success(),
        "smelt build (target={target}) failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

// ─── Shared workspace files ───────────────────────────────────────────────────

/// A small deterministic CSV seed that mirrors the `bronze.raw_events` schema
/// used by the real web_analytics workspace.  Five rows with known payloads so
/// the output is predictable.
const RAW_EVENTS_CSV: &str = "\
event_id,device_id,user_id,seconds_in_day,payload,event_date\n\
1,101,201,3600,\"{\"\"event_name\"\":\"\"page_view\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com/home\"\"}\",2026-03-19\n\
2,102,202,7200,\"{\"\"event_name\"\":\"\"click\"\",\"\"platform\"\":\"\"ios\"\",\"\"url\"\":\"\"https://example.com/product\"\"}\",2026-03-19\n\
3,103,203,10800,\"{\"\"event_name\"\":\"\"purchase\"\",\"\"platform\"\":\"\"android\"\",\"\"url\"\":\"\"https://example.com/checkout\"\"}\",2026-03-20\n\
4,104,204,14400,\"{\"\"event_name\"\":\"\"scroll\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com/product\"\"}\",2026-03-20\n\
5,105,205,18000,\"{\"\"event_name\"\":\"\"error\"\",\"\"platform\"\":\"\"ios\"\",\"\"url\"\":\"\"https://example.com/account\"\"}\",2026-03-21\n\
";

/// `parse_event_payload` function declaration (identical to the one in
/// `examples/web_analytics/functions/parse_event_payload.sql`).
const PARSE_EVENT_PAYLOAD_FN: &str = "\
---
backends: [duckdb]
---
smelt.define parse_event_payload(
    payload_json: Expr<Text>
) -> Expr<Struct<{event_name: Text, platform: Text, url: Text}>> AS (
    {
        json_extract_string(payload_json, '$.event_name') AS event_name,
        json_extract_string(payload_json, '$.platform') AS platform,
        json_extract_string(payload_json, '$.url') AS url
    }
)
";

/// Original `events_parsed.sql` — inlines the three `json_extract_string`
/// calls directly (no function call).
const EVENTS_PARSED_ORIGINAL: &str = "\
---
materialization: table
---
SELECT
    event_id,
    device_id,
    user_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    json_extract_string(payload, '$.event_name') AS event_name,
    json_extract_string(payload, '$.platform') AS platform,
    json_extract_string(payload, '$.url') AS url
FROM smelt.raw_events
ORDER BY event_id
";

/// Refactored `events_parsed.sql` — calls `smelt.functions.parse_event_payload`
/// with `.*` projection in place of the three inlined calls.
const EVENTS_PARSED_REFACTORED: &str = "\
---
materialization: table
---
SELECT
    event_id,
    device_id,
    user_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    smelt.functions.parse_event_payload(payload).*
FROM smelt.raw_events
ORDER BY event_id
";

fn smelt_yml_for(name: &str, db_path: &Path) -> String {
    format!(
        "name: {name}
version: 1
paths:
  - models
  - seeds
targets:
  dev:
    type: duckdb
    database: {db}
    schema: main
default_materialization: view
",
        db = db_path.display()
    )
}

// ─── Row type for the events_parsed output ────────────────────────────────────

#[derive(Debug, PartialEq)]
struct EventsParsedRow {
    event_id: i64,
    device_id: i64,
    user_id: Option<i64>,
    event_name: String,
    platform: String,
    url: String,
}

fn query_events_parsed(db_path: &Path) -> Vec<EventsParsedRow> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT event_id, device_id, user_id, event_name, platform, url \
             FROM main.events_parsed ORDER BY event_id",
        )
        .expect("prepare events_parsed query");
    stmt.query_map([], |row| {
        Ok(EventsParsedRow {
            event_id: row.get::<_, i64>(0)?,
            device_id: row.get::<_, i64>(1)?,
            user_id: row.get::<_, Option<i64>>(2)?,
            event_name: row.get::<_, String>(3)?,
            platform: row.get::<_, String>(4)?,
            url: row.get::<_, String>(5)?,
        })
    })
    .expect("query events_parsed rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect rows")
}

// ─── Snapshot test ────────────────────────────────────────────────────────────

/// Build the *original* `events_parsed.sql` (three inlined `json_extract_string`
/// calls) and the *refactored* variant (`smelt.functions.parse_event_payload(payload).*`)
/// against the same deterministic CSV seed, then assert the resulting row sets
/// are identical.
///
/// This is the TDD gate for Phase 1.3 of
/// `docs/plans/20260519-functions-meta-1-call-expansion.md`.  The test is RED
/// until `examples/web_analytics/models/silver/events_parsed.sql` is rewritten
/// to use the function call.
#[test]
fn events_parsed_rowset_equivalent_after_refactor() {
    // ── Build original variant ──────────────────────────────────────────────
    let tmp_orig = TempDir::new().expect("tempdir");
    let db_orig = tmp_orig.path().join("orig.duckdb");
    let yml_orig = smelt_yml_for("snapshot_original", &db_orig);
    write_workspace(
        tmp_orig.path(),
        &[
            ("smelt.yml", yml_orig.as_str()),
            ("seeds/raw_events.csv", RAW_EVENTS_CSV),
            ("functions/parse_event_payload.sql", PARSE_EVENT_PAYLOAD_FN),
            ("models/events_parsed.sql", EVENTS_PARSED_ORIGINAL),
        ],
    );
    run_smelt_build(tmp_orig.path(), "dev");
    let rows_original = query_events_parsed(&db_orig);

    // ── Build refactored variant ────────────────────────────────────────────
    let tmp_ref = TempDir::new().expect("tempdir");
    let db_ref = tmp_ref.path().join("ref.duckdb");
    let yml_ref = smelt_yml_for("snapshot_refactored", &db_ref);
    write_workspace(
        tmp_ref.path(),
        &[
            ("smelt.yml", yml_ref.as_str()),
            ("seeds/raw_events.csv", RAW_EVENTS_CSV),
            ("functions/parse_event_payload.sql", PARSE_EVENT_PAYLOAD_FN),
            ("models/events_parsed.sql", EVENTS_PARSED_REFACTORED),
        ],
    );
    run_smelt_build(tmp_ref.path(), "dev");
    let rows_refactored = query_events_parsed(&db_ref);

    // ── Assert byte-equal row sets ─────────────────────────────────────────
    assert_eq!(
        rows_original.len(),
        5,
        "original: expected 5 rows, got: {rows_original:?}"
    );
    assert_eq!(
        rows_refactored.len(),
        5,
        "refactored: expected 5 rows, got: {rows_refactored:?}"
    );
    assert_eq!(
        rows_original, rows_refactored,
        "Row sets differ between original (inlined json_extract_string) \
         and refactored (parse_event_payload.* projection).\n\
         original:   {rows_original:?}\n\
         refactored: {rows_refactored:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase 1.4: sessions.sql refactor snapshot
// ─────────────────────────────────────────────────────────────────────────────
//
// The workspace mirrors the real `web_analytics` layout but with a small
// deterministic CSV seed instead of Parquet data:
//
//   seeds/raw_events.csv               — 8 events, 2 devices, designed to
//                                         exercise both the inactivity-gap and
//                                         platform-change session boundaries.
//   models/silver/events_parsed.sql    — inline (same in both variants).
//   functions/sessionize.sql           — the canonical smelt.define declaration.
//   models/silver/sessions.sql         — ORIGINAL (3-CTE) or REFACTORED (fn call).
//
// Seed design:
//   Device 1 ('web' throughout):
//     event 1 at  3600 s  (01:00:00)
//     event 2 at  5200 s  (01:26:40  — 26.7 min gap → same session 0)
//     event 3 at  8000 s  (02:13:20  — 46.7 min gap → NEW session 1)
//     event 4 at  9000 s  (02:30:00  — 16.7 min gap → same session 1)
//   Device 2 (platform boundary web→ios):
//     event 5 at  3600 s  (01:00:00, platform='web')
//     event 6 at  4200 s  (01:10:00, platform='ios' — only 10 min but platform
//                           changed → NEW session 1)
//     event 7 at  4800 s  (01:20:00, platform='ios'  — same session 1)
//     event 8 at  5400 s  (01:30:00, platform='ios'  — same session 1)
//
// Expected sessions (ORDER BY session_id):
//   device=1, seq=0, events 1+2, count=2, platform='web'
//   device=1, seq=1, events 3+4, count=2, platform='web'
//   device=2, seq=0, event 5,    count=1, platform='web'
//   device=2, seq=1, events 6+7+8, count=3, platform='ios'

/// Seed CSV shared by both session workspace variants.  8 rows, two devices.
///
/// Device 1 — inactivity-gap boundary (day-level granularity in the seed data;
///   `event_ts` is a TIMESTAMP, so the gap comparison operates on the actual
///   time-of-day delta between events):
///   events 1 + 2 on 2026-03-19 → session 0 (gap between same-day events = 0).
///   events 3 + 4 on 2026-03-20 → session 1 (gap = 1 day >> 30 min).
///
/// Device 2 — boundary triggered by both gap AND platform change:
///   event 5 on 2026-03-19 (web) → session 0.
///   events 6-8 on 2026-03-20 (ios) → session 1 (1 day gap + platform web→ios).
///   Multiple same-date events within a session (7 and 8) are safe because
///   the gap comparison ties at 0 and the platform is unchanged.
const SESSIONS_SEED_CSV: &str = "\
event_id,device_id,user_id,seconds_in_day,payload,event_date\n\
1,1,101,3600,\"{\"\"event_name\"\":\"\"page_view\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-19\n\
2,1,101,5200,\"{\"\"event_name\"\":\"\"click\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-19\n\
3,1,101,8000,\"{\"\"event_name\"\":\"\"purchase\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-20\n\
4,1,101,9000,\"{\"\"event_name\"\":\"\"logout\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-20\n\
5,2,102,3600,\"{\"\"event_name\"\":\"\"page_view\"\",\"\"platform\"\":\"\"web\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-19\n\
6,2,102,4200,\"{\"\"event_name\"\":\"\"click\"\",\"\"platform\"\":\"\"ios\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-20\n\
7,2,102,4800,\"{\"\"event_name\"\":\"\"scroll\"\",\"\"platform\"\":\"\"ios\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-20\n\
8,2,102,5400,\"{\"\"event_name\"\":\"\"logout\"\",\"\"platform\"\":\"\"ios\"\",\"\"url\"\":\"\"https://example.com\"\"}\",2026-03-20\n\
";

/// `events_parsed.sql` — shared by both variants; inlines the three
/// json_extract_string calls (no function) to isolate the sessions comparison.
const EVENTS_PARSED_INLINE: &str = "\
---
materialization: table
---
SELECT
    event_id,
    device_id,
    user_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    json_extract_string(payload, '$.event_name') AS event_name,
    json_extract_string(payload, '$.platform') AS platform,
    json_extract_string(payload, '$.url') AS url
FROM smelt.raw_events
";

/// `sessionize` function declaration — the canonical signature.
///
/// The body uses `epoch_us()` microsecond arithmetic rather than INTERVAL
/// subtraction so the same expression works whether `ts_col` is a DATE or a
/// TIMESTAMP (BIGINT - BIGINT), independent of the caller's column type.
///
/// The `gap` parameter is in microseconds (default 30 minutes = 1 800 000 000 μs).
///
/// The body uses a CTE (_lagged) to resolve the LAG window functions before
/// the SUM counter, avoiding DuckDB's prohibition on nested window functions.
/// A CTE is used instead of an inline subquery so smelt's body checker can
/// see the _lagged columns (_smelt_prev_ts_us, _smelt_prev_platform) in scope.
const SESSIONIZE_FN: &str = "\
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
    ts_col: Expr<Timestamp>,
    platform_col: Expr<Text>,
    gap: Expr<BigInt> = 30 * 60 * 1000000
) -> TableExpr AS (
    WITH _lagged AS (
        SELECT
            *,
            LAG(epoch_us(ts_col)) OVER (PARTITION BY partition_col ORDER BY ts_col) AS _smelt_prev_ts_us,
            LAG(platform_col) OVER (PARTITION BY partition_col ORDER BY ts_col) AS _smelt_prev_platform
        FROM source
    )
    SELECT
        *,
        SUM(
            CASE
                WHEN epoch_us(ts_col) - _smelt_prev_ts_us > gap
                  OR _smelt_prev_platform != platform_col
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY partition_col ORDER BY ts_col) AS session_seq
    FROM _lagged
)
";

/// Original `sessions.sql` — three-CTE inlined pipeline using epoch_us
/// microsecond arithmetic to handle the BIGINT/INTERVAL comparison.
const SESSIONS_ORIGINAL: &str = "\
---
materialization: table
---
WITH lagged AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        LAG(epoch_us(event_ts)) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev_ts_us,
        LAG(platform) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev_platform
    FROM smelt.silver.events_parsed
),
sessionized AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        SUM(
            CASE
                WHEN epoch_us(event_ts) - prev_ts_us > 30 * 60 * 1000000
                  OR prev_platform != platform
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY device_id ORDER BY event_ts) AS session_seq
    FROM lagged
),
with_start_date AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        session_seq,
        CAST(FIRST_VALUE(event_ts) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS DATE) AS session_start_date
    FROM sessionized
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_seq AS VARCHAR), '-', CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM with_start_date
GROUP BY device_id, session_seq, session_start_date
";

/// Refactored `sessions.sql` — delegates sessionization to
/// `smelt.functions.sessionize` with named arguments and an explicit `AS s`
/// alias on the FROM-position call.
const SESSIONS_REFACTORED: &str = "\
---
materialization: table
---
WITH sessionized AS (
    SELECT *
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    ) AS s
),
with_start_date AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        session_seq,
        CAST(FIRST_VALUE(event_ts) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS DATE) AS session_start_date
    FROM sessionized
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_seq AS VARCHAR), '-', CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM with_start_date
GROUP BY device_id, session_seq, session_start_date
";

fn smelt_yml_for_sessions(name: &str, db_path: &Path) -> String {
    format!(
        "name: {name}
version: 1
paths:
  - models
  - seeds
targets:
  dev:
    type: duckdb
    database: {db}
    schema: main
default_materialization: view
",
        db = db_path.display()
    )
}

// ─── Row type for the sessions output ─────────────────────────────────────────

#[derive(Debug, PartialEq, Clone)]
struct SessionRow {
    session_id: String,
    device_id: i64,
    session_seq: i64,
    session_start_date: String,
    event_count: i64,
    platform: String,
}

fn query_sessions(db_path: &Path) -> Vec<SessionRow> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT session_id, device_id, session_seq, \
             session_start_date::VARCHAR, event_count, platform \
             FROM main.silver_sessions ORDER BY session_id",
        )
        .expect("prepare sessions query");
    stmt.query_map([], |row| {
        Ok(SessionRow {
            session_id: row.get::<_, String>(0)?,
            device_id: row.get::<_, i64>(1)?,
            session_seq: row.get::<_, i64>(2)?,
            session_start_date: row.get::<_, String>(3)?,
            event_count: row.get::<_, i64>(4)?,
            platform: row.get::<_, String>(5)?,
        })
    })
    .expect("query sessions rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect rows")
}

// ─── Snapshot test ─────────────────────────────────────────────────────────────

/// Build the *original* `sessions.sql` (three-CTE inlined pipeline) and the
/// *refactored* variant (`smelt.functions.sessionize` with named args) against
/// the same deterministic CSV seed.  Assert the resulting row sets are identical.
///
/// The seed exercises two session boundaries: an inactivity-gap boundary for
/// device 1 (> 30 min between events 2 and 3) and a platform-change boundary
/// for device 2 (web → ios between events 5 and 6), producing 4 sessions total.
#[test]
fn sessions_rowset_equivalent_after_refactor() {
    // ── Build original variant ──────────────────────────────────────────────
    let tmp_orig = TempDir::new().expect("tempdir");
    let db_orig = tmp_orig.path().join("orig.duckdb");
    let yml_orig = smelt_yml_for_sessions("sessions_original", &db_orig);
    write_workspace(
        tmp_orig.path(),
        &[
            ("smelt.yml", yml_orig.as_str()),
            ("seeds/raw_events.csv", SESSIONS_SEED_CSV),
            ("functions/sessionize.sql", SESSIONIZE_FN),
            ("models/silver/events_parsed.sql", EVENTS_PARSED_INLINE),
            ("models/silver/sessions.sql", SESSIONS_ORIGINAL),
        ],
    );
    run_smelt_build(tmp_orig.path(), "dev");
    let rows_original = query_sessions(&db_orig);

    // ── Build refactored variant ────────────────────────────────────────────
    let tmp_ref = TempDir::new().expect("tempdir");
    let db_ref = tmp_ref.path().join("ref.duckdb");
    let yml_ref = smelt_yml_for_sessions("sessions_refactored", &db_ref);
    write_workspace(
        tmp_ref.path(),
        &[
            ("smelt.yml", yml_ref.as_str()),
            ("seeds/raw_events.csv", SESSIONS_SEED_CSV),
            ("functions/sessionize.sql", SESSIONIZE_FN),
            ("models/silver/events_parsed.sql", EVENTS_PARSED_INLINE),
            ("models/silver/sessions.sql", SESSIONS_REFACTORED),
        ],
    );
    run_smelt_build(tmp_ref.path(), "dev");
    let rows_refactored = query_sessions(&db_ref);

    // ── Structural checks ───────────────────────────────────────────────────
    assert_eq!(
        rows_original.len(),
        4,
        "original: expected 4 sessions, got: {rows_original:#?}"
    );
    assert_eq!(
        rows_refactored.len(),
        4,
        "refactored: expected 4 sessions, got: {rows_refactored:#?}"
    );

    // ── Device 1: two time-gap-based sessions ───────────────────────────────
    let dev1_orig: Vec<_> = rows_original.iter().filter(|r| r.device_id == 1).collect();
    let dev1_ref: Vec<_> = rows_refactored
        .iter()
        .filter(|r| r.device_id == 1)
        .collect();
    assert_eq!(
        dev1_orig.len(),
        2,
        "original: expected 2 sessions for device 1"
    );
    assert_eq!(
        dev1_ref.len(),
        2,
        "refactored: expected 2 sessions for device 1"
    );
    assert_eq!(dev1_orig[0].session_seq, 0);
    assert_eq!(dev1_orig[0].event_count, 2);
    assert_eq!(dev1_orig[0].platform, "web");
    assert_eq!(dev1_orig[1].session_seq, 1);
    assert_eq!(dev1_orig[1].event_count, 2);
    assert_eq!(dev1_orig[1].platform, "web");

    // ── Device 2: platform-change boundary ─────────────────────────────────
    let dev2_orig: Vec<_> = rows_original.iter().filter(|r| r.device_id == 2).collect();
    let dev2_ref: Vec<_> = rows_refactored
        .iter()
        .filter(|r| r.device_id == 2)
        .collect();
    assert_eq!(
        dev2_orig.len(),
        2,
        "original: expected 2 sessions for device 2"
    );
    assert_eq!(
        dev2_ref.len(),
        2,
        "refactored: expected 2 sessions for device 2"
    );
    assert_eq!(dev2_orig[0].session_seq, 0);
    assert_eq!(dev2_orig[0].event_count, 1);
    assert_eq!(dev2_orig[0].platform, "web");
    assert_eq!(dev2_orig[1].session_seq, 1);
    assert_eq!(dev2_orig[1].event_count, 3);
    assert_eq!(dev2_orig[1].platform, "ios");

    // ── Row-set equality between original and refactored ───────────────────
    assert_eq!(
        rows_original, rows_refactored,
        "Row sets differ between original (3-CTE inlined) \
         and refactored (smelt.functions.sessionize named-arg call).\n\
         original:   {rows_original:#?}\n\
         refactored: {rows_refactored:#?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// sessions.sql refactor without explicit alias snapshot
// ─────────────────────────────────────────────────────────────────────────────
//
// Same workspace as the Phase 1.4 test above, but the `sessions.sql` body
// omits the `AS s` alias on the FROM-position function call.  The compiler
// must synthesise a derived-table alias so DuckDB accepts the query.
// The rowset must be identical to the alias-carrying variant above.

/// `sessions.sql` with the explicit `AS s` alias removed — the compiler
/// synthesises the alias automatically.
const SESSIONS_REFACTORED_NO_ALIAS: &str = "\
---
materialization: table
---
WITH sessionized AS (
    SELECT *
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
),
with_start_date AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        session_seq,
        CAST(FIRST_VALUE(event_ts) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS DATE) AS session_start_date
    FROM sessionized
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_seq AS VARCHAR), '-', CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM with_start_date
GROUP BY device_id, session_seq, session_start_date
";

/// Build the *with-alias* and *without-alias* variants of `sessions.sql`
/// side by side.  Assert that both build and execute successfully and that
/// the resulting rowsets are identical.
///
/// Assert the `sessionize` call in FROM position produces an identical rowset
/// whether the caller writes `AS s` explicitly or omits it (the printer
/// synthesises a unique alias automatically when none is supplied).
#[test]
fn sessions_rowset_equivalent_without_explicit_alias() {
    // ── Build with-alias variant (explicit `AS s`) ─────────────────────────
    let tmp_with = TempDir::new().expect("tempdir");
    let db_with = tmp_with.path().join("with_alias.duckdb");
    let yml_with = smelt_yml_for_sessions("sessions_with_alias", &db_with);
    write_workspace(
        tmp_with.path(),
        &[
            ("smelt.yml", yml_with.as_str()),
            ("seeds/raw_events.csv", SESSIONS_SEED_CSV),
            ("functions/sessionize.sql", SESSIONIZE_FN),
            ("models/silver/events_parsed.sql", EVENTS_PARSED_INLINE),
            ("models/silver/sessions.sql", SESSIONS_REFACTORED),
        ],
    );
    run_smelt_build(tmp_with.path(), "dev");
    let rows_with_alias = query_sessions(&db_with);

    // ── Build without-alias variant ────────────────────────────────────────
    let tmp_no = TempDir::new().expect("tempdir");
    let db_no = tmp_no.path().join("no_alias.duckdb");
    let yml_no = smelt_yml_for_sessions("sessions_no_alias", &db_no);
    write_workspace(
        tmp_no.path(),
        &[
            ("smelt.yml", yml_no.as_str()),
            ("seeds/raw_events.csv", SESSIONS_SEED_CSV),
            ("functions/sessionize.sql", SESSIONIZE_FN),
            ("models/silver/events_parsed.sql", EVENTS_PARSED_INLINE),
            ("models/silver/sessions.sql", SESSIONS_REFACTORED_NO_ALIAS),
        ],
    );
    run_smelt_build(tmp_no.path(), "dev");
    let rows_no_alias = query_sessions(&db_no);

    // ── Assert byte-equal row sets ─────────────────────────────────────────
    assert_eq!(
        rows_with_alias.len(),
        4,
        "with-alias: expected 4 sessions, got: {rows_with_alias:#?}"
    );
    assert_eq!(
        rows_no_alias.len(),
        4,
        "without-alias: expected 4 sessions, got: {rows_no_alias:#?}"
    );
    assert_eq!(
        rows_with_alias, rows_no_alias,
        "Row sets differ between with-alias and without-alias variants.\n\
         with-alias:    {rows_with_alias:#?}\n\
         without-alias: {rows_no_alias:#?}"
    );
}
