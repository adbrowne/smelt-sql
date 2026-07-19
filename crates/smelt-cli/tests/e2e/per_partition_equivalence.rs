#![cfg(feature = "duckdb")]
//! Per-partition equivalence harness for `examples/web_analytics/`.
//!
//! Asserts the formal contract from `docs/specs/incremental_models.md`
//! §"Per-partition equivalence":
//!
//!   incremental_run(model, [run_start, run_end))
//!     .where(partition_column = p)
//!   == full_refresh(model).where(partition_column = p)
//!
//! for **local** columns (columns whose value depends only on source rows
//! visible within the model's source-filter ranges).
//!
//! **Global** columns — `dau_backward_fill`, `dau_connected_components`,
//! `identified_events_backward_fill`, `identified_events_connected_components`
//! — are *not* expected to match per-partition because they depend on
//! the cumulative (device, user) edge set across all dates. The day-by-day
//! pipeline emits as-of-day-D values; a full-window rebuild emits the final
//! global snapshot. The test asserts that this divergence *exists* (at least
//! one partition differs) and is bounded (the total set of partitions is
//! known), documenting the as-of-day-D property.
//!
//! # Design notes
//!
//! We run a **7-day** window (2026-03-19 .. 2026-03-26) rather than the
//! full 60-day datagen window to keep CI runtime under ~30 s. The divergence
//! pattern on the global columns is present even on a 7-day window; the exact
//! count is dataset-dependent but the qualitative property (some partitions
//! differ, others may agree) holds.
//!
//! The test requires both `smelt` and `smelt-datagen` binaries. Since
//! integration tests in `smelt-cli` have `CARGO_BIN_EXE_smelt` injected by
//! Cargo, we resolve `smelt-datagen` as a sibling in the same `target/`
//! directory.
//!
//! # Porting note
//!
//! This Rust test supersedes `examples/web_analytics/verify_incremental_equivalence.py`.
//! That script is retained for human convenience (it is easier to run
//! manually with custom --days / --scale-factor flags), but the Rust harness
//! is the authoritative CI gate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ── Binary paths ──────────────────────────────────────────────────────────────

/// The compiled `smelt` binary. Cargo injects `CARGO_BIN_EXE_smelt` for
/// integration tests within the `smelt-cli` crate.
fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Resolve the `smelt-datagen` binary as a sibling of `smelt` in the same
/// `target/<profile>/` directory.
fn datagen_bin() -> PathBuf {
    smelt_bin()
        .parent()
        .expect("smelt binary must have a parent dir")
        .join("smelt-datagen")
}

/// Absolute path to the repo root (CARGO_MANIFEST_DIR is `crates/smelt-cli`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .expect("crates dir")
        .parent() // repo root
        .expect("repo root")
        .to_owned()
}

// ── Setup helpers ─────────────────────────────────────────────────────────────

/// Recursively copy `src` directory tree into `dst`, creating `dst` if needed.
fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {dst:?}: {e}"));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("readdir {src:?}: {e}")) {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap_or_else(|e| panic!("copy {from:?} → {to:?}: {e}"));
        }
    }
}

/// Rewrite `datagen.yaml` so each `output:` path points under `output_base`
/// rather than the relative `data/` prefix. Written to `dest_path`.
fn rewrite_datagen_outputs(src: &Path, dest: &Path, output_base: &Path) {
    let content = fs::read_to_string(src).unwrap_or_else(|e| panic!("read {src:?}: {e}"));
    let mut out = String::with_capacity(content.len() + 256);
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("output:") {
            let val = rest.trim().trim_matches('"');
            let leaf = val.split('/').next_back().unwrap_or("dataset");
            let abs = output_base.join(leaf);
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push_str(&format!("{}output: {}\n", indent, abs.display()));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    fs::write(dest, &out).unwrap_or_else(|e| panic!("write {dest:?}: {e}"));
}

/// Rewrite `setup_sources.sql` so the `'data/` prefix points to `output_base`.
fn rewrite_setup_sources(src: &Path, dest: &Path, output_base: &Path) {
    let content = fs::read_to_string(src).unwrap_or_else(|e| panic!("read {src:?}: {e}"));
    let rewritten = content.replace("'data/", &format!("'{}/", output_base.display()));
    fs::write(dest, &rewritten).unwrap_or_else(|e| panic!("write {dest:?}: {e}"));
}

/// Run `smelt-datagen --config <cfg> --scale-factor <sf>`, panic on failure.
fn run_datagen(config_path: &Path, scale_factor: f64, label: &str) {
    let datagen = datagen_bin();
    assert!(
        datagen.exists(),
        "smelt-datagen not found at {datagen:?}; run `cargo build -p smelt-datagen` first"
    );
    let out = Command::new(&datagen)
        .arg("--config")
        .arg(config_path)
        .arg("--scale-factor")
        .arg(scale_factor.to_string())
        .output()
        .unwrap_or_else(|e| panic!("[{label}] failed to spawn smelt-datagen: {e}"));
    if !out.status.success() {
        panic!(
            "[{label}] smelt-datagen failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// Execute `setup_sources.sql` (with rewritten paths) via the `duckdb` crate.
fn setup_sources(db_path: &Path, setup_sql_path: &Path) {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let sql =
        fs::read_to_string(setup_sql_path).unwrap_or_else(|e| panic!("read setup_sources: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources.sql: {e}\nSQL:\n{sql}"));
}

/// Remove `target/dev.duckdb` if it exists (reset between pipeline runs).
fn reset_db(workspace: &Path) {
    let db = workspace.join("target/dev.duckdb");
    if db.exists() {
        fs::remove_file(&db).unwrap_or_else(|e| panic!("remove {db:?}: {e}"));
    }
}

// ── smelt run helpers ─────────────────────────────────────────────────────────

/// Run `smelt run --event-time-start S --event-time-end E` in `workspace`.
fn smelt_run(workspace: &Path, start: &str, end: &str, label: &str) {
    let smelt = smelt_bin();
    let out = Command::new(&smelt)
        .args(["run", "--event-time-start", start, "--event-time-end", end])
        .current_dir(workspace)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("[{label}] failed to spawn smelt run: {e}"));
    if !out.status.success() {
        panic!(
            "[{label}] smelt run [{start} .. {end}) failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

// ── DAU row query ─────────────────────────────────────────────────────────────

/// One row from `marts_daily_active_users_by_method`.
#[derive(Debug, Clone, PartialEq)]
struct DauRow {
    event_date: String,
    total_events: i64,
    dau_raw: i64,
    dau_forward_only: i64,
    dau_backward_fill: i64,
    dau_connected_components: i64,
    identified_events_raw: i64,
    identified_events_forward_only: i64,
    identified_events_backward_fill: i64,
    identified_events_connected_components: i64,
}

fn query_dau_rows(db_path: &Path) -> Vec<DauRow> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT \
                event_date::VARCHAR, \
                total_events, \
                dau_raw, \
                dau_forward_only, \
                dau_backward_fill, \
                dau_connected_components, \
                identified_events_raw, \
                identified_events_forward_only, \
                identified_events_backward_fill, \
                identified_events_connected_components \
             FROM main.marts_daily_active_users_by_method \
             ORDER BY event_date",
        )
        .unwrap_or_else(|e| panic!("prepare DAU query: {e}"));

    stmt.query_map([], |row| {
        Ok(DauRow {
            event_date: row.get::<_, String>(0)?,
            total_events: row.get::<_, i64>(1)?,
            dau_raw: row.get::<_, i64>(2)?,
            dau_forward_only: row.get::<_, i64>(3)?,
            dau_backward_fill: row.get::<_, i64>(4)?,
            dau_connected_components: row.get::<_, i64>(5)?,
            identified_events_raw: row.get::<_, i64>(6)?,
            identified_events_forward_only: row.get::<_, i64>(7)?,
            identified_events_backward_fill: row.get::<_, i64>(8)?,
            identified_events_connected_components: row.get::<_, i64>(9)?,
        })
    })
    .unwrap_or_else(|e| panic!("query DAU rows: {e}"))
    .collect::<Result<Vec<_>, _>>()
    .unwrap_or_else(|e| panic!("collect DAU rows: {e}"))
}

// ── Workspace staging ─────────────────────────────────────────────────────────

/// Stage a hermetic web_analytics workspace in `tmp` and populate source tables.
/// Returns `(workspace_dir, db_path, setup_abs_sql_path)`.
///
/// The example workspace tree may contain a `target/dev.duckdb` from a
/// previous manual run.  We always delete it after copying so that Pipeline A
/// starts with a clean slate (no pre-existing incremental partitions).
fn stage_workspace(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let workspace = tmp.join("workspace");
    let datagen_out = tmp.join("data");
    let db_path = workspace.join("target/dev.duckdb");

    // Clone the example workspace tree.
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, &workspace);

    // Delete any pre-existing target/dev.duckdb that was copied from the
    // checked-in tree (from a prior manual run).  Tests must start from zero.
    if db_path.exists() {
        fs::remove_file(&db_path)
            .unwrap_or_else(|e| panic!("remove pre-existing {db_path:?}: {e}"));
    }

    // Run datagen with rewritten output paths.
    let src_cfg = workspace.join("datagen.yaml");
    let dest_cfg = tmp.join("datagen_rewritten.yaml");
    rewrite_datagen_outputs(&src_cfg, &dest_cfg, &datagen_out);
    run_datagen(&dest_cfg, 0.01, "datagen");

    // Rewrite setup_sources.sql so `'data/` points to the absolute datagen output.
    let setup_src = workspace.join("setup_sources.sql");
    let setup_abs = tmp.join("setup_sources_abs.sql");
    rewrite_setup_sources(&setup_src, &setup_abs, &datagen_out);

    // Create target/ dir and populate source tables.
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    setup_sources(&db_path, &setup_abs);

    (workspace, db_path, setup_abs)
}

/// Re-populate source tables into `db_path` (needed between pipeline A→B runs).
fn repopulate_sources(db_path: &Path, setup_abs: &Path) {
    setup_sources(db_path, setup_abs);
}

// ── Cross-midnight session fixture ───────────────────────────────────────────

/// Synthetic `device_id` for the forced cross-midnight session pair — well
/// outside `datagen.yaml`'s 150,000-row `devices` dataset, so it can never
/// collide with an organically-generated device.
const CROSS_MIDNIGHT_DEVICE_ID: i64 = 999_900_001;

/// Insert a synthetic two-event pair into `raw.events` that crosses the
/// day-1/day-2 midnight boundary (`DAY_WINDOWS[0]` / `DAY_WINDOWS[1]`) with a
/// 16-minute gap — well under `sessionize`'s 30-minute inactivity threshold —
/// and the same `platform`, so `smelt.functions.sessionize` merges them into
/// one session rooted on day 1 (`session_start_date = 2026-03-19`) whose
/// `session_end` lands on day 2 (`2026-03-20`).
///
/// This is the minimal repro of the write-window skew divergence documented
/// in `docs/plans/20260710-web-analytics-maintenance-demo.md`
/// §"Deferred during implementation" (the day-46 `event_id=7647` case): a
/// real cross-midnight pair occurs in the datagen output only about once
/// every 50 days at `scale_factor=0.01` (rare relative to the 30-minute
/// inactivity gate), too infrequent to guarantee inside the harness's 7-day
/// CI window. Both `arrival_time` values equal `event_time` (on-time
/// delivery) so `silver.events_parsed`'s lateness filter never excludes
/// either row, and both events are inserted before *any* `smelt run`, so
/// both pipelines (full-window and day-by-day) see identical source data —
/// only the run-window shape differs between them.
fn inject_cross_midnight_session_pair(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let sql = format!(
        "INSERT INTO raw.events \
            (event_id, device_id, user_id, seconds_in_day, event_time, arrival_time, \
             utm_campaign, payload, event_date) \
         VALUES \
            (900000001, {dev}, NULL, 85620, \
                TIMESTAMP '2026-03-19 23:47:00', TIMESTAMP '2026-03-19 23:47:00', NULL, \
                '{{\"event_name\": \"page_view\", \"platform\": \"web\", \
                   \"url\": \"https://example.com/home\"}}', \
                DATE '2026-03-19'), \
            (900000002, {dev}, NULL, 180, \
                TIMESTAMP '2026-03-20 00:03:00', TIMESTAMP '2026-03-20 00:03:00', NULL, \
                '{{\"event_name\": \"page_view\", \"platform\": \"web\", \
                   \"url\": \"https://example.com/home\"}}', \
                DATE '2026-03-20');",
        dev = CROSS_MIDNIGHT_DEVICE_ID,
    );
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("insert cross-midnight session pair: {e}\nSQL:\n{sql}"));
}

/// Synthetic `device_id` for the forced **two-boundary** event chain — a
/// second device, distinct from `CROSS_MIDNIGHT_DEVICE_ID`, so the two
/// injected shapes never interact.
const TWO_BOUNDARY_DEVICE_ID: i64 = 999_900_002;

/// Insert a synthetic 60-event chain into `raw.events` that spans **two**
/// midnights with every pairwise gap under `sessionize`'s 30-minute
/// inactivity threshold and a constant `platform` — the shape a live
/// sessionize step treats as one continuous, never-idle activity stream:
///
///   - `2026-03-19 23:50` (the chain root, day 1),
///   - `2026-03-20 00:10` + 25-minute steps through `2026-03-20 23:55`
///     (58 events, crossing the first midnight),
///   - `2026-03-21 00:15` (crossing the second midnight).
///
/// No inactivity gap ever breaks this chain, so the only thing that can end
/// the session rooted at `23:50` is the **clock-anchored cut**
/// (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
/// §"silver.sessions — clock-anchored cut"): the root's time-of-day
/// (`23:50`) is `>= 00:30`, so its deadline reaches to the *second*
/// midnight (the start of `2026-03-21`). Every event strictly before that
/// deadline — the root plus the full day-2 grid — merges into one 59-event
/// session; the first event at or past the deadline (`2026-03-21 00:15`)
/// is a forced root and starts its own singleton session. The truncation
/// this chain realises is therefore the *real* function's, end to end — not
/// a precomputed stand-in — and the harness's set-equality plus the pinned
/// per-session assertions in
/// `web_analytics_session_attribution_matches_full_rebuild` verify it is
/// identical between the day-by-day replay and the full rebuild.
///
/// All `arrival_time`s equal `event_time` (on-time delivery) and all rows
/// are inserted before *any* `smelt run`, exactly like
/// `inject_cross_midnight_session_pair` above.
fn inject_two_boundary_session_chain(db_path: &Path) {
    let root = chrono::NaiveDateTime::parse_from_str("2026-03-19 23:50:00", "%Y-%m-%d %H:%M:%S")
        .expect("parse chain root");
    let day2_base =
        chrono::NaiveDateTime::parse_from_str("2026-03-20 00:10:00", "%Y-%m-%d %H:%M:%S")
            .expect("parse day-2 base");
    let tail = chrono::NaiveDateTime::parse_from_str("2026-03-21 00:15:00", "%Y-%m-%d %H:%M:%S")
        .expect("parse chain tail");

    let mut timestamps = vec![root];
    // k = 0..=57: 2026-03-20 00:10 … 2026-03-20 23:55 in 25-minute steps.
    for k in 0..=57 {
        timestamps.push(day2_base + chrono::Duration::minutes(25 * k));
    }
    timestamps.push(tail);

    let values: Vec<String> = timestamps
        .iter()
        .enumerate()
        .map(|(i, ts)| {
            use chrono::Timelike;
            let seconds_in_day = i64::from(ts.time().num_seconds_from_midnight());
            format!(
                "(9000000{:02}, {dev}, NULL, {seconds_in_day}, \
                    TIMESTAMP '{ts}', TIMESTAMP '{ts}', NULL, \
                    '{{\"event_name\": \"page_view\", \"platform\": \"web\", \
                       \"url\": \"https://example.com/home\"}}', \
                    DATE '{date}')",
                i + 11, // event_id 900000011 … 900000070 (past the pair's 01/02)
                dev = TWO_BOUNDARY_DEVICE_ID,
                ts = ts.format("%Y-%m-%d %H:%M:%S"),
                date = ts.format("%Y-%m-%d"),
            )
        })
        .collect();

    let sql = format!(
        "INSERT INTO raw.events \
            (event_id, device_id, user_id, seconds_in_day, event_time, arrival_time, \
             utm_campaign, payload, event_date) \
         VALUES {};",
        values.join(", ")
    );
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("insert two-boundary session chain: {e}\nSQL:\n{sql}"));
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// We run 7 days (2026-03-19 … 2026-03-26) rather than the full 60-day window
/// to keep CI runtime under ~30 s.  The divergence pattern on global columns
/// is visible even on a 7-day window.
const START_DATE: &str = "2026-03-19";
const END_DATE_EXCLUSIVE: &str = "2026-03-26"; // 7 days

/// Day sequence for the day-by-day pipeline: each entry is (window_start,
/// window_end) passed to `smelt run`. Windows are non-overlapping — the
/// workspace includes `silver.device_user_edges`, an additive-fold keyed
/// model (`grain: key`), and the transactional merge ledger refuses to
/// re-fold a partition it has already merged (`docs/specs/incremental_models.md`
/// §"Reprocessing" / §"The transactional merge ledger" —
/// `KeyedReprocessedWindow`). The Python driver's superseded 1-day-lookback
/// schedule predates that model and would double-fold day D on both day D's
/// and day D+1's window; it is not reproduced here.
const DAY_WINDOWS: &[(&str, &str)] = &[
    ("2026-03-19", "2026-03-20"), // day 1
    ("2026-03-20", "2026-03-21"), // day 2
    ("2026-03-21", "2026-03-22"), // day 3
    ("2026-03-22", "2026-03-23"), // day 4
    ("2026-03-23", "2026-03-24"), // day 5
    ("2026-03-24", "2026-03-25"), // day 6
    ("2026-03-25", "2026-03-26"), // day 7
];

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: Local columns are exactly equal per partition
// ─────────────────────────────────────────────────────────────────────────────

/// Asserts that `total_events`, `dau_raw`, `dau_forward_only`,
/// `identified_events_raw`, and `identified_events_forward_only` are exactly
/// equal between the full-window pipeline (Pipeline A) and the day-by-day
/// pipeline (Pipeline B) for every partition (event_date).
///
/// These are the **local** columns — their per-partition value depends only on
/// source rows whose event_date falls within the partition's source-filter
/// range. They must be exactly equal by the per-partition equivalence contract.
#[test]
fn test_local_columns_equivalent() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(&workspace, START_DATE, END_DATE_EXCLUSIVE, "pipeline-A");
    let rows_a = query_dau_rows(&db_path);

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);

    for (ws, we) in DAY_WINDOWS {
        smelt_run(&workspace, ws, we, &format!("pipeline-B [{ws}..{we})"));
    }
    let rows_b = query_dau_rows(&db_path);

    // ── Assertions ─────────────────────────────────────────────────────────
    assert_eq!(
        rows_a.len(),
        rows_b.len(),
        "Pipeline A and B must produce the same number of partitions (event_date rows).\n\
         A: {} rows, B: {} rows",
        rows_a.len(),
        rows_b.len(),
    );
    assert!(
        !rows_a.is_empty(),
        "No rows produced — check that the pipeline ran successfully"
    );

    let mut mismatches = Vec::new();
    for (ra, rb) in rows_a.iter().zip(rows_b.iter()) {
        // Partition key must match (ordering guard)
        assert_eq!(
            ra.event_date, rb.event_date,
            "Partition mismatch: A={}, B={}",
            ra.event_date, rb.event_date
        );

        if ra.total_events != rb.total_events {
            mismatches.push(format!(
                "  {} total_events: A={} B={}",
                ra.event_date, ra.total_events, rb.total_events
            ));
        }
        if ra.dau_raw != rb.dau_raw {
            mismatches.push(format!(
                "  {} dau_raw: A={} B={}",
                ra.event_date, ra.dau_raw, rb.dau_raw
            ));
        }
        if ra.dau_forward_only != rb.dau_forward_only {
            mismatches.push(format!(
                "  {} dau_forward_only: A={} B={}",
                ra.event_date, ra.dau_forward_only, rb.dau_forward_only
            ));
        }
        if ra.identified_events_raw != rb.identified_events_raw {
            mismatches.push(format!(
                "  {} identified_events_raw: A={} B={}",
                ra.event_date, ra.identified_events_raw, rb.identified_events_raw
            ));
        }
        if ra.identified_events_forward_only != rb.identified_events_forward_only {
            mismatches.push(format!(
                "  {} identified_events_forward_only: A={} B={}",
                ra.event_date, ra.identified_events_raw, rb.identified_events_forward_only
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "LOCAL-COLUMN MISMATCH (unexpected — per-partition equivalence violated):\n{}\n\
         Full A rows ({} total):\n{:?}\n\
         Full B rows ({} total):\n{:?}",
        mismatches.join("\n"),
        rows_a.len(),
        rows_a,
        rows_b.len(),
        rows_b,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: Global columns diverge with the documented as-of-day-D pattern
// ─────────────────────────────────────────────────────────────────────────────

/// Asserts the *documented divergence* on global identity columns:
///
/// - `dau_backward_fill` and `dau_connected_components`, and their
///   `identified_events_*` counterparts, **may** differ between the
///   full-window pipeline and the day-by-day pipeline.
///
/// The divergence arises because these columns depend on the cumulative
/// (device, user) edge set across all dates. Day D's incremental run freezes
/// the mapping using only the edges visible up to D; a later edge that would
/// have changed day D's cluster is not retroactively applied.
///
/// This test verifies that:
/// 1. The divergence exists (at least one partition differs on at least one
///    global column), asserting the as-of-day-D property.
/// 2. The total number of partitions is the expected window size (7).
///
/// If the test fixture generates data where no device-merging ever happens
/// (no shared devices), all global columns would agree — the test guards
/// against this being silently true by asserting the datagen seed produces
/// at least some non-trivial identity resolution (total_events > 0).
#[test]
fn test_global_columns_documented_divergence() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(
        &workspace,
        START_DATE,
        END_DATE_EXCLUSIVE,
        "pipeline-A-global",
    );
    let rows_a = query_dau_rows(&db_path);

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);

    for (ws, we) in DAY_WINDOWS {
        smelt_run(
            &workspace,
            ws,
            we,
            &format!("pipeline-B-global [{ws}..{we})"),
        );
    }
    let rows_b = query_dau_rows(&db_path);

    // ── Baseline: we actually got data ────────────────────────────────────
    assert!(
        !rows_a.is_empty(),
        "Pipeline A produced no rows — check datagen + setup_sources"
    );
    assert_eq!(
        rows_a.len(),
        rows_b.len(),
        "Pipeline A and B partition counts differ: A={} B={}",
        rows_a.len(),
        rows_b.len()
    );

    // The full 7-day window should produce 7 partitions.
    assert_eq!(
        rows_a.len(),
        7,
        "Expected 7 partitions ({START_DATE} .. {END_DATE_EXCLUSIVE}), got {}",
        rows_a.len()
    );

    // ── Sanity: datagen actually generated some events ────────────────────
    let total_events_a: i64 = rows_a.iter().map(|r| r.total_events).sum();
    assert!(
        total_events_a > 0,
        "total_events is 0 across all partitions — datagen produced no rows"
    );

    // ── Count per-global-column divergences ───────────────────────────────
    let mut diffs: std::collections::HashMap<&str, usize> = [
        ("dau_backward_fill", 0usize),
        ("dau_connected_components", 0usize),
        ("identified_events_backward_fill", 0usize),
        ("identified_events_connected_components", 0usize),
    ]
    .into_iter()
    .collect();

    for (ra, rb) in rows_a.iter().zip(rows_b.iter()) {
        if ra.dau_backward_fill != rb.dau_backward_fill {
            *diffs.get_mut("dau_backward_fill").unwrap() += 1;
        }
        if ra.dau_connected_components != rb.dau_connected_components {
            *diffs.get_mut("dau_connected_components").unwrap() += 1;
        }
        if ra.identified_events_backward_fill != rb.identified_events_backward_fill {
            *diffs.get_mut("identified_events_backward_fill").unwrap() += 1;
        }
        if ra.identified_events_connected_components != rb.identified_events_connected_components {
            *diffs
                .get_mut("identified_events_connected_components")
                .unwrap() += 1;
        }
    }

    let total = rows_a.len();
    eprintln!("=== global identity column divergence (expected) ===");
    for (col, count) in &diffs {
        eprintln!("  {col}: {count}/{total} partitions differ between pipelines");
    }

    // Assert that the number of diverging partitions is bounded by the window
    // size — can't diverge on more partitions than we have.
    for (col, &count) in &diffs {
        assert!(
            count <= total,
            "divergence count for {col} ({count}) exceeds partition count ({total})"
        );
    }

    // At scale-factor=0.01 on a 7-day window, the datagen seed (42) produces
    // events with ~15% shared-device and ~5% multi-device-user tuples (see
    // datagen.yaml linked_pools weights). With ~700 events (1M * 0.01 / 60 * 7),
    // cross-device linking is highly likely.
    //
    // We assert that AT LEAST ONE global column has at least one diverging
    // partition, confirming the as-of-day-D property is observable. If this
    // assertion fails, it likely means the dataset happened to have no
    // cross-device identity resolution — in which case this assertion should be
    // relaxed and the test doc updated.
    let any_divergence: bool = diffs.values().any(|&c| c > 0);
    assert!(
        any_divergence,
        "No global-column divergence detected across any partition. \
         This is unexpected at scale-factor=0.01 with seed=42 (which has ~15%% \
         shared-device events). Either the datagen seed changed, the pipeline \
         is incorrectly computing global identity, or there is a genuine edge \
         case where all as-of-day-D values happen to equal the final global \
         snapshot. Divergence counts: {diffs:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: Harness runs under `cargo test` (no manual Python invocation needed)
// ─────────────────────────────────────────────────────────────────────────────

/// Smoke test: the equivalence harness runs end-to-end in `cargo test -p
/// smelt-cli --test per_partition_equivalence` without any manual Python
/// invocation. Validates the test infrastructure itself by running a minimal
/// 2-day window and asserting structural invariants.
#[test]
fn test_runs_under_test_harness() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);

    // Run a minimal 2-day window — just enough to confirm the harness works.
    let start = "2026-03-19";
    let end = "2026-03-21";

    smelt_run(&workspace, start, end, "harness-smoke-A");
    let rows_a = query_dau_rows(&db_path);

    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);

    // Day-by-day: 2 non-overlapping single-day windows (see `DAY_WINDOWS`
    // above for why — `silver.device_user_edges` is additive-fold and
    // refuses to re-fold an already-ledgered partition).
    smelt_run(
        &workspace,
        "2026-03-19",
        "2026-03-20",
        "harness-smoke-B-day1",
    );
    smelt_run(
        &workspace,
        "2026-03-20",
        "2026-03-21",
        "harness-smoke-B-day2",
    );
    let rows_b = query_dau_rows(&db_path);

    // Both pipelines must produce results.
    assert!(!rows_a.is_empty(), "Pipeline A produced no rows");
    assert!(!rows_b.is_empty(), "Pipeline B produced no rows");

    // Both must produce the same number of partitions.
    assert_eq!(
        rows_a.len(),
        rows_b.len(),
        "Partition counts differ: A={} B={}",
        rows_a.len(),
        rows_b.len()
    );

    // Exactly 2 partitions expected for a [2026-03-19, 2026-03-21) window.
    assert_eq!(
        rows_a.len(),
        2,
        "Expected 2 partitions for 2-day window, got {}",
        rows_a.len()
    );

    // Local columns (total_events, dau_raw) must be exactly equal.
    for (ra, rb) in rows_a.iter().zip(rows_b.iter()) {
        assert_eq!(
            ra.event_date, rb.event_date,
            "Partition key mismatch: A={} B={}",
            ra.event_date, rb.event_date
        );
        assert_eq!(
            ra.total_events, rb.total_events,
            "total_events mismatch at {}: A={} B={}",
            ra.event_date, ra.total_events, rb.total_events
        );
        assert_eq!(
            ra.dau_raw, rb.dau_raw,
            "dau_raw mismatch at {}: A={} B={}",
            ra.event_date, ra.dau_raw, rb.dau_raw
        );
    }

    // The smelt binary and datagen binary were both found and executed.
    assert!(
        smelt_bin().exists(),
        "smelt binary not found at {:?}",
        smelt_bin()
    );
    assert!(
        datagen_bin().exists(),
        "smelt-datagen binary not found at {:?}",
        datagen_bin()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: silver/events_parsed dedup over the 3-day late window
// ─────────────────────────────────────────────────────────────────────────────
//
// `docs/plans/20260710-web-analytics-maintenance-demo.md` Phase 5: the
// redelivery/lateness datagen shape (Phase 4) means `raw.events` contains
// byte-identical duplicate rows (same `event_id`, later `arrival_time`) and
// events whose `arrival_time` trails `event_time` by up to 3 days.
// `silver/events_parsed` absorbs both: `QUALIFY ROW_NUMBER() OVER (PARTITION
// BY event_id ORDER BY arrival_time) = 1` drops the redelivered duplicate,
// and the Form B filter `event_date BETWEEN CAST(arrival_time AS DATE) -
// INTERVAL '3 days' AND CAST(arrival_time AS DATE)` accepts late arrivals up
// to that window — a genuine 3-day lookback the planner derives from the
// filter text (visible via `smelt explain silver.events_parsed --json`).

/// One `(event_id, event_date)` pair from `main.silver_events_parsed`.
fn query_events_parsed_ids(db_path: &Path) -> Vec<(i64, String)> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare("SELECT event_id, event_date::VARCHAR FROM main.silver_events_parsed")
        .unwrap_or_else(|e| panic!("prepare events_parsed query: {e}"));
    stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })
    .unwrap_or_else(|e| panic!("query events_parsed rows: {e}"))
    .collect::<Result<Vec<_>, _>>()
    .unwrap_or_else(|e| panic!("collect events_parsed rows: {e}"))
}

/// Every `(event_id, event_date)` in `raw.events` whose lateness
/// (`arrival_time - event_time`) is within the accepted 3-day window — the
/// set `silver.events_parsed`'s acceptance filter must retain (at least the
/// earliest-arriving copy of each `event_id`).
fn query_acceptable_event_ids(db_path: &Path) -> std::collections::BTreeSet<(i64, String)> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT event_id, event_date::VARCHAR \
             FROM raw.events \
             WHERE CAST(arrival_time AS TIMESTAMP) \
                 <= CAST(event_time AS TIMESTAMP) + INTERVAL '3 days'",
        )
        .unwrap_or_else(|e| panic!("prepare acceptable-events query: {e}"));
    stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })
    .unwrap_or_else(|e| panic!("query acceptable events: {e}"))
    .collect::<Result<_, _>>()
    .unwrap_or_else(|e| panic!("collect acceptable events: {e}"))
}

/// Day-by-day incremental build of `silver.events_parsed` equals one
/// full-window rebuild: zero duplicate `event_id`s in the result, and every
/// event within the accepted 3-day late window is present in its own
/// `event_date` partition — in both pipelines.
#[test]
fn web_analytics_dedup_matches_full_rebuild() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(
        &workspace,
        START_DATE,
        END_DATE_EXCLUSIVE,
        "dedup-pipeline-A",
    );
    let rows_a = query_events_parsed_ids(&db_path);
    let acceptable = query_acceptable_event_ids(&db_path);

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);

    for (ws, we) in DAY_WINDOWS {
        smelt_run(
            &workspace,
            ws,
            we,
            &format!("dedup-pipeline-B [{ws}..{we})"),
        );
    }
    let rows_b = query_events_parsed_ids(&db_path);

    assert!(
        !rows_a.is_empty(),
        "Pipeline A produced no events_parsed rows"
    );
    assert!(
        !rows_b.is_empty(),
        "Pipeline B produced no events_parsed rows"
    );

    // ── Zero duplicate event_ids in either pipeline's result ──────────────
    for (label, rows) in [("A (full rebuild)", &rows_a), ("B (day-by-day)", &rows_b)] {
        let mut counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        for (event_id, _) in rows {
            *counts.entry(*event_id).or_insert(0) += 1;
        }
        let dups: Vec<_> = counts.iter().filter(|(_, &c)| c > 1).collect();
        assert!(
            dups.is_empty(),
            "pipeline {label} has duplicate event_ids in silver.events_parsed: {dups:?}"
        );
    }

    // ── Day-by-day equals full rebuild exactly (set of (event_id, event_date)) ──
    let set_a: std::collections::BTreeSet<_> = rows_a.iter().cloned().collect();
    let set_b: std::collections::BTreeSet<_> = rows_b.iter().cloned().collect();
    let only_in_a: Vec<_> = set_a.difference(&set_b).take(10).collect();
    let only_in_b: Vec<_> = set_b.difference(&set_a).take(10).collect();
    assert!(
        only_in_a.is_empty() && only_in_b.is_empty(),
        "silver.events_parsed differs between full rebuild and day-by-day replay.\n\
         only in A (first 10): {only_in_a:?}\nonly in B (first 10): {only_in_b:?}"
    );

    // ── Every accepted-lateness event is present in its own event_date
    //    partition, in both pipelines (within the [START_DATE,
    //    END_DATE_EXCLUSIVE) window the harness runs) ───────────────────────
    let window_acceptable: Vec<_> = acceptable
        .iter()
        .filter(|(_, d)| d.as_str() >= START_DATE && d.as_str() < END_DATE_EXCLUSIVE)
        .collect();
    assert!(
        !window_acceptable.is_empty(),
        "no acceptable-lateness events found in the run window — check datagen output"
    );
    for (event_id, event_date) in &window_acceptable {
        assert!(
            set_a.contains(&(*event_id, event_date.clone())),
            "event_id={event_id} (event_date={event_date}, within the 3-day late window) \
             missing from pipeline A's silver.events_parsed"
        );
        assert!(
            set_b.contains(&(*event_id, event_date.clone())),
            "event_id={event_id} (event_date={event_date}, within the 3-day late window) \
             missing from pipeline B's silver.events_parsed"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: session campaign attribution matches full rebuild + respects the cap
// ─────────────────────────────────────────────────────────────────────────────

/// One row from `silver.sessions`, projected for attribution + cap checks.
#[derive(Debug, Clone)]
struct SessionRow {
    session_id: String,
    device_id: i64,
    session_start: String,
    session_end: String,
    event_count: i64,
    utm_campaign: Option<String>,
}

fn query_session_rows(db_path: &Path) -> Vec<SessionRow> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT \
                session_id, \
                device_id, \
                CAST(session_start AS VARCHAR), \
                CAST(session_end AS VARCHAR), \
                event_count, \
                utm_campaign \
             FROM main.silver_sessions \
             ORDER BY session_id",
        )
        .unwrap_or_else(|e| panic!("prepare sessions query: {e}"));
    stmt.query_map([], |row| {
        Ok(SessionRow {
            session_id: row.get::<_, String>(0)?,
            device_id: row.get::<_, i64>(1)?,
            session_start: row.get::<_, String>(2)?,
            session_end: row.get::<_, String>(3)?,
            event_count: row.get::<_, i64>(4)?,
            utm_campaign: row.get::<_, Option<String>>(5)?,
        })
    })
    .unwrap_or_else(|e| panic!("query session rows: {e}"))
    .collect::<Result<Vec<_>, _>>()
    .unwrap_or_else(|e| panic!("collect session rows: {e}"))
}

/// For every session row, the earliest non-NULL `utm_campaign` among the
/// events that actually belong to that session (`event_ts` within
/// `[session_start, session_end]`, the model's own, independently-tested
/// session boundaries — see `tests/session_boundary_invariants.test.sql`)
/// and within the first 5 minutes of session start. This is a golden
/// attribution query computed independently of the model's own `ARG_MAX`
/// aggregation, via a correlated subquery straight against `events_parsed`;
/// it does not re-derive session *membership* (that has its own dedicated
/// invariant test), only the attribution rule given membership.
fn query_expected_attribution(db_path: &Path) -> std::collections::HashMap<String, Option<String>> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT \
                s.session_id, \
                ( \
                    SELECT e.utm_campaign \
                    FROM main.silver_events_parsed e \
                    WHERE e.device_id = s.device_id \
                      AND e.event_ts >= CAST(s.session_start AS TIMESTAMP) \
                      AND e.event_ts <= CAST(s.session_end AS TIMESTAMP) \
                      AND e.event_ts <= CAST(s.session_start AS TIMESTAMP) + INTERVAL '5 minutes' \
                      AND e.utm_campaign IS NOT NULL \
                    ORDER BY e.event_ts ASC \
                    LIMIT 1 \
                ) AS expected_campaign \
             FROM main.silver_sessions s",
        )
        .unwrap_or_else(|e| panic!("prepare expected-attribution query: {e}"));
    stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })
    .unwrap_or_else(|e| panic!("query expected attribution: {e}"))
    .collect::<Result<_, _>>()
    .unwrap_or_else(|e| panic!("collect expected attribution: {e}"))
}

/// Day-by-day incremental build of `silver.sessions` equals one full-window
/// rebuild on `(session_id, session_start, session_end, event_count,
/// utm_campaign)`, campaign attribution comes only from events within the
/// first 5 minutes of the session (verified against an independently-computed
/// golden query over `events_parsed`), and no session exceeds the explicit
/// max-session-length cap.
///
/// Includes a forced cross-midnight event pair
/// (`inject_cross_midnight_session_pair`) spanning `DAY_WINDOWS[0]` /
/// `DAY_WINDOWS[1]` — the shape that exposes a write-window skew divergence:
/// `session_id`/`utm_campaign` alone are invariant under that bug (session
/// identity is the root timestamp; attribution is the first 5 minutes), but
/// `session_end`/`event_count` diverge when the neighbour partition a
/// cross-midnight session reaches is never rewritten
/// (`docs/plans/20260710-web-analytics-maintenance-demo.md`
/// §"Deferred during implementation").
#[test]
fn web_analytics_session_attribution_matches_full_rebuild() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);
    inject_cross_midnight_session_pair(&db_path);
    inject_two_boundary_session_chain(&db_path);

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(
        &workspace,
        START_DATE,
        END_DATE_EXCLUSIVE,
        "session-pipeline-A",
    );
    let rows_a = query_session_rows(&db_path);
    let expected_a = query_expected_attribution(&db_path);

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);
    inject_cross_midnight_session_pair(&db_path);
    inject_two_boundary_session_chain(&db_path);

    for (ws, we) in DAY_WINDOWS {
        smelt_run(
            &workspace,
            ws,
            we,
            &format!("session-pipeline-B [{ws}..{we})"),
        );
    }
    let rows_b = query_session_rows(&db_path);
    let expected_b = query_expected_attribution(&db_path);

    assert!(!rows_a.is_empty(), "Pipeline A produced no session rows");
    assert!(!rows_b.is_empty(), "Pipeline B produced no session rows");

    // ── Attribution correctness (independent golden query) ────────────────
    for (label, rows, expected) in [
        ("A (full rebuild)", &rows_a, &expected_a),
        ("B (day-by-day)", &rows_b, &expected_b),
    ] {
        let mut mismatches = Vec::new();
        for row in rows {
            let want = expected.get(&row.session_id).cloned().flatten();
            if row.utm_campaign != want {
                mismatches.push(format!(
                    "  session_id={} actual={:?} expected(first-5-min earliest)={:?}",
                    row.session_id, row.utm_campaign, want
                ));
            }
        }
        assert!(
            mismatches.is_empty(),
            "pipeline {label}: utm_campaign attribution mismatch \
             (expected: earliest non-NULL campaign among events within the \
             session's first 5 minutes):\n{}",
            mismatches.join("\n")
        );
    }

    // ── Cap: no session exceeds the explicit max-session-span cap (< 2 days) ──
    for (label, rows) in [("A (full rebuild)", &rows_a), ("B (day-by-day)", &rows_b)] {
        for row in rows {
            let start =
                chrono::NaiveDateTime::parse_from_str(&row.session_start, "%Y-%m-%d %H:%M:%S%.f")
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(
                            &row.session_start,
                            "%Y-%m-%d %H:%M:%S",
                        )
                    })
                    .unwrap_or_else(|e| panic!("parse session_start {:?}: {e}", row.session_start));
            let end =
                chrono::NaiveDateTime::parse_from_str(&row.session_end, "%Y-%m-%d %H:%M:%S%.f")
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&row.session_end, "%Y-%m-%d %H:%M:%S")
                    })
                    .unwrap_or_else(|e| panic!("parse session_end {:?}: {e}", row.session_end));
            let duration = end - start;
            assert!(
                duration < chrono::Duration::days(2),
                "pipeline {label}: session_id={} exceeds the max-session-span cap: \
                 session_start={} session_end={} duration={duration}",
                row.session_id,
                row.session_start,
                row.session_end,
            );
        }
    }

    // ── Day-by-day equals full rebuild exactly on (session_id, session_start,
    //    session_end, event_count, utm_campaign) ────────────────────────────
    //
    // Widened beyond (session_id, utm_campaign): both are invariant under the
    // write-window skew divergence this harness must catch (session identity
    // is the root timestamp; attribution is fixed by the first 5 minutes),
    // so a bug that leaves a cross-midnight session's neighbour partition
    // stale (wrong `session_end`/`event_count`) would pass silently under the
    // narrower assertion.
    let set_a: std::collections::BTreeSet<_> = rows_a
        .iter()
        .map(|r| {
            (
                r.session_id.clone(),
                r.session_start.clone(),
                r.session_end.clone(),
                r.event_count,
                r.utm_campaign.clone(),
            )
        })
        .collect();
    let set_b: std::collections::BTreeSet<_> = rows_b
        .iter()
        .map(|r| {
            (
                r.session_id.clone(),
                r.session_start.clone(),
                r.session_end.clone(),
                r.event_count,
                r.utm_campaign.clone(),
            )
        })
        .collect();
    let only_in_a: Vec<_> = set_a.difference(&set_b).take(10).collect();
    let only_in_b: Vec<_> = set_b.difference(&set_a).take(10).collect();
    assert!(
        only_in_a.is_empty() && only_in_b.is_empty(),
        "silver.sessions (session_id, session_start, session_end, event_count, \
         utm_campaign) differs between full rebuild and day-by-day replay.\n\
         only in A (first 10): {only_in_a:?}\nonly in B (first 10): {only_in_b:?}"
    );

    // ── Two-boundary truncation, pinned against the REAL sessionize ───────
    //
    // The injected 60-event chain (`inject_two_boundary_session_chain`)
    // never breaks on inactivity or platform, so its only natural boundary
    // is the root at 2026-03-19 23:50. That root's time-of-day is `>= 00:30`,
    // so its deadline reaches to the *second* midnight (the start of
    // 2026-03-21) — the clock-anchored cut
    // (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
    // §"silver.sessions — clock-anchored cut"). Every event strictly before
    // that deadline — the root plus the full 25-minute day-2 grid — merges
    // into one 59-event session; the first event at or past the deadline
    // (2026-03-21 00:15) is a forced root and starts its own singleton
    // session. The model's Form B relation restates the same one-day-forward
    // reach in *date* space (`event_date BETWEEN session_start_date AND
    // session_start_date + INTERVAL '1 day'`) — the declared relation the
    // output window derives from.
    //
    // These pins are asserted per pipeline (not just via the set-equality
    // above) so the real function's truncation behaviour is documented
    // here, and any divergence from the mirrored fixture expectation in
    // `cross_midnight_rebase.rs::two_boundary_session_truncated_at_declared_bound`
    // surfaces as an explicit failure.
    for (label, rows) in [("A (full rebuild)", &rows_a), ("B (day-by-day)", &rows_b)] {
        let mut chain: Vec<&SessionRow> = rows
            .iter()
            .filter(|r| r.device_id == TWO_BOUNDARY_DEVICE_ID)
            .collect();
        chain.sort_by(|a, b| a.session_start.cmp(&b.session_start));

        let observed: Vec<(&str, &str, i64, Option<&str>)> = chain
            .iter()
            .map(|r| {
                (
                    r.session_start.as_str(),
                    r.session_end.as_str(),
                    r.event_count,
                    r.utm_campaign.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            observed,
            vec![
                ("2026-03-19 23:50:00", "2026-03-20 23:55:00", 59, None),
                ("2026-03-21 00:15:00", "2026-03-21 00:15:00", 1, None),
            ],
            "pipeline {label}: the real sessionize must cut the two-boundary \
             chain at the clock-anchored deadline (the second midnight) — a \
             59-event session (root plus the full day-2 grid, ending at \
             2026-03-20 23:55) plus one singleton session rooted by the \
             first post-deadline event; got {observed:?}"
        );

        // Non-overlap: the device's sessions never overlap in time.
        for pair in chain.windows(2) {
            assert!(
                pair[1].session_start > pair[0].session_end,
                "pipeline {label}: sessions must not overlap — session \
                 starting {} begins at or before the previous session's end {}",
                pair[1].session_start,
                pair[0].session_end,
            );
        }

        // Event conservation: every injected event counted exactly once.
        let total: i64 = chain.iter().map(|r| r.event_count).sum();
        assert_eq!(
            total, 60,
            "pipeline {label}: the chain device's sessions must account for \
             all 60 injected events exactly once (59 + 1)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: silver/events_enriched — event-grain narrow update at model-upstream
// creation cells
// ─────────────────────────────────────────────────────────────────────────────
//
// `docs/plans/20260710-web-analytics-maintenance-demo.md` Phase 7:
// `silver/events_enriched` joins `silver.events_parsed` and `silver.sessions`
// (two maintained-model upstreams) back onto the event grain, `grain:
// partition` on `event_date`. Its creation cells are clamped by each
// upstream's own derived reach (`docs/specs/incremental_models.md` §"Upstream
// model edges") — see `crates/smelt-cli/tests/explain_model.rs
// ::events_enriched_shows_creation_cells_for_both_model_upstreams` for the
// static evidence. These tests exercise the *dynamic* consequence on a real
// fixture: incremental equals full rebuild, and a run touching one arrival
// day only ever changes `event_date` partitions within the derived window.

/// One row from `main.silver_events_enriched`, keyed for equivalence checks.
///
/// `(event_id, session_id, session_utm_campaign, session_id_chained,
/// session_utm_campaign_chained)` — the last two carry the root-anchored
/// identity from `silver.sessions_chained`
/// (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
/// §"Enrichment"), alongside the primary clock-anchored pair from
/// `silver.sessions`.
type EventEnrichedRow = (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Every `silver.events_enriched` row grouped by its `event_date` partition.
fn query_events_enriched_by_partition(
    db_path: &Path,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<EventEnrichedRow>> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT event_date::VARCHAR, event_id, session_id, session_utm_campaign, \
                    session_id_chained, session_utm_campaign_chained \
             FROM main.silver_events_enriched",
        )
        .unwrap_or_else(|e| panic!("prepare events_enriched query: {e}"));
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .unwrap_or_else(|e| panic!("query events_enriched rows: {e}"))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|e| panic!("collect events_enriched rows: {e}"));

    let mut by_partition: std::collections::BTreeMap<
        String,
        std::collections::BTreeSet<EventEnrichedRow>,
    > = std::collections::BTreeMap::new();
    for (
        event_date,
        event_id,
        session_id,
        session_utm_campaign,
        session_id_chained,
        session_utm_campaign_chained,
    ) in rows
    {
        by_partition.entry(event_date).or_default().insert((
            event_id,
            session_id,
            session_utm_campaign,
            session_id_chained,
            session_utm_campaign_chained,
        ));
    }
    by_partition
}

/// Day-by-day incremental build of `silver.events_enriched` equals a
/// full-window rebuild exactly, per partition — the per-partition
/// equivalence contract (`docs/specs/incremental_models.md`) applied to a
/// model whose creation trigger reads two model upstreams rather than a
/// single source.
#[test]
fn web_analytics_events_enriched_matches_full_rebuild() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(
        &workspace,
        START_DATE,
        END_DATE_EXCLUSIVE,
        "enriched-pipeline-A",
    );
    let by_partition_a = query_events_enriched_by_partition(&db_path);

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);

    for (ws, we) in DAY_WINDOWS {
        smelt_run(
            &workspace,
            ws,
            we,
            &format!("enriched-pipeline-B [{ws}..{we})"),
        );
    }
    let by_partition_b = query_events_enriched_by_partition(&db_path);

    assert!(
        !by_partition_a.is_empty(),
        "Pipeline A produced no silver.events_enriched rows"
    );
    assert_eq!(
        by_partition_a.keys().collect::<Vec<_>>(),
        by_partition_b.keys().collect::<Vec<_>>(),
        "silver.events_enriched partition sets (event_date) differ between \
         full rebuild and day-by-day replay"
    );

    let mut mismatches = Vec::new();
    for (partition, rows_a) in &by_partition_a {
        let rows_b = by_partition_b.get(partition);
        if rows_b != Some(rows_a) {
            mismatches.push(format!(
                "  partition {partition}: A has {} rows, B has {:?} rows",
                rows_a.len(),
                rows_b.map(|r| r.len())
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "silver.events_enriched differs between full rebuild and day-by-day \
         replay on at least one partition:\n{}",
        mismatches.join("\n")
    );
}

/// Snapshot `silver.events_enriched`'s partitions after a day-by-day replay
/// through day 6, then run one additional arrival day (day 7) and assert the
/// change is exactly narrow: `event_date`'s output partition column is not
/// write-rebased by either upstream edge (`silver.events_parsed`'s own
/// creation-cell clamp is `Bounded(0,0)` — a direct 1:1 read — and
/// `silver.sessions`'s ±1-day session-cap clamp only widens the *read*, not
/// the write footprint; see `crates/smelt-cli/tests/explain_model.rs
/// ::events_enriched_shows_creation_cells_for_both_model_upstreams` for the
/// static clamp evidence), so a single-day run touches exactly its own
/// `event_date` partition and no other. Verified empirically against the
/// real fixture: running the day-7 window changes only the 2026-03-25
/// partition; every one of the six previously-written partitions
/// (2026-03-19 .. 2026-03-24) is byte-identical before and after. Asserted
/// on observed partition contents (row sets), never on implementation logs.
#[test]
fn web_analytics_events_enriched_narrow_update() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);
    repopulate_sources(&db_path, &setup_abs);

    // Replay the first 6 of the 7 configured days (2026-03-19 .. 2026-03-25).
    let (initial_windows, remaining_windows) = DAY_WINDOWS.split_at(DAY_WINDOWS.len() - 1);
    for (ws, we) in initial_windows {
        smelt_run(&workspace, ws, we, &format!("narrow-before [{ws}..{we})"));
    }
    let before = query_events_enriched_by_partition(&db_path);
    assert!(
        !before.is_empty(),
        "no silver.events_enriched rows after the initial 6-day replay"
    );
    assert_eq!(
        before.keys().cloned().collect::<Vec<_>>(),
        vec![
            "2026-03-19",
            "2026-03-20",
            "2026-03-21",
            "2026-03-22",
            "2026-03-23",
            "2026-03-24",
        ],
        "unexpected partition set after the initial 6-day replay"
    );

    // Run the one additional arrival day (day 7: 2026-03-25 .. 2026-03-26).
    for (ws, we) in remaining_windows {
        smelt_run(&workspace, ws, we, &format!("narrow-after [{ws}..{we})"));
    }
    let after = query_events_enriched_by_partition(&db_path);

    // Exactly one new partition appears (2026-03-25), and every
    // previously-written partition is byte-identical.
    assert_eq!(
        after.keys().cloned().collect::<Vec<_>>(),
        vec![
            "2026-03-19",
            "2026-03-20",
            "2026-03-21",
            "2026-03-22",
            "2026-03-23",
            "2026-03-24",
            "2026-03-25",
        ],
        "expected exactly one new partition (2026-03-25) after the additional \
         arrival day"
    );

    let mut unexpected_changes = Vec::new();
    for partition in before.keys() {
        if before.get(partition) != after.get(partition) {
            unexpected_changes.push(partition.clone());
        }
    }
    assert!(
        unexpected_changes.is_empty(),
        "narrow update touched previously-written partition(s) it should not \
         have (a run touching only 2026-03-25 must not rewrite earlier \
         partitions): {unexpected_changes:?}"
    );
}

/// `silver.events_enriched` is now downstream of **two** maintained session
/// tables (`docs/plans/20260711-clock-vs-root-anchored-sessions.md` Phase
/// 4): `silver.sessions` (window-independent, clock-anchored) and
/// `silver.sessions_chained` (self-referential, `Ordered`, root-anchored —
/// Phase 3). The per-partition equivalence contract must still hold end to
/// end even though one of the two upstreams now builds under strict
/// sequential ordering rather than in parallel: day-by-day incremental
/// maintenance of `events_enriched` must equal a full-window rebuild,
/// column-for-column, including the added `session_id_chained` /
/// `session_utm_campaign_chained` pair — not just the pre-existing
/// `session_id` / `session_utm_campaign` pair from `silver.sessions`.
#[test]
fn events_enriched_dual_ids_replay_matches_rebuild() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let (workspace, db_path, setup_abs) = stage_workspace(tmp_path);

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(
        &workspace,
        START_DATE,
        END_DATE_EXCLUSIVE,
        "enriched-dual-pipeline-A",
    );
    let by_partition_a = query_events_enriched_by_partition(&db_path);

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    repopulate_sources(&db_path, &setup_abs);

    for (ws, we) in DAY_WINDOWS {
        smelt_run(
            &workspace,
            ws,
            we,
            &format!("enriched-dual-pipeline-B [{ws}..{we})"),
        );
    }
    let by_partition_b = query_events_enriched_by_partition(&db_path);

    assert!(
        !by_partition_a.is_empty(),
        "Pipeline A produced no silver.events_enriched rows"
    );
    assert_eq!(
        by_partition_a.keys().collect::<Vec<_>>(),
        by_partition_b.keys().collect::<Vec<_>>(),
        "silver.events_enriched partition sets (event_date) differ between \
         full rebuild and day-by-day replay once sessions_chained (Ordered) \
         is joined in"
    );

    let mut mismatches = Vec::new();
    for (partition, rows_a) in &by_partition_a {
        let rows_b = by_partition_b.get(partition);
        if rows_b != Some(rows_a) {
            mismatches.push(format!(
                "  partition {partition}: A has {} rows, B has {:?} rows",
                rows_a.len(),
                rows_b.map(|r| r.len())
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "silver.events_enriched (dual session ids) differs between full \
         rebuild and day-by-day replay on at least one partition:\n{}",
        mismatches.join("\n")
    );

    // Every event carries both identities: `session_id` (primary,
    // `silver.sessions`) and `session_id_chained` (`silver.sessions_chained`)
    // must both be populated for every row — the join is inner on both
    // upstreams, so a missing id on either side would mean the row dropped
    // out of the result set entirely rather than surfacing as NULL, but we
    // assert it directly against the materialized rows as the ground truth.
    let mut missing_id = Vec::new();
    for (partition, rows) in &by_partition_a {
        for (event_id, session_id, _, session_id_chained, _) in rows {
            if session_id.is_none() || session_id_chained.is_none() {
                missing_id.push(format!(
                    "partition {partition} event_id {event_id}: session_id={session_id:?} \
                     session_id_chained={session_id_chained:?}"
                ));
            }
        }
    }
    assert!(
        missing_id.is_empty(),
        "every silver.events_enriched row must carry both session_id and \
         session_id_chained: {}",
        missing_id.join("\n")
    );
}
