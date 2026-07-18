#![cfg(feature = "duckdb")]
//! End-to-end redelivery equivalence for `examples/web_analytics`'s composed
//! `silver.events_deduped` model (`docs/plans/20260715-composed-axes-
//! conditional-maintenance.md` Phase W1).
//!
//! `silver.events_deduped` is the flagship composed shape
//! (`docs/specs/incremental_models.md` §"Key temporal locality (the
//! time-partitioned output)"): key-addressed (one row per `event_id`) *and*
//! time-partitioned (`first_seen_date`), admitted via **route 3**
//! (recurrence-bounded) over `sources.raw.events`'s declared
//! `mutation_profile.key_recurrence`. `datagen.yaml`'s `redelivery:` block
//! produces the duplicate-delivery storms this route absorbs — ~2% of
//! events arrive twice, byte-identical except `arrival_time`.
//!
//! This harness asserts the formal contract those routes exist to uphold:
//!
//!   incremental_state(silver.events_deduped) == full_refresh(sources.raw.events)
//!
//! for any run-window decomposition of a fixed source snapshot — driving a
//! real datagen dataset (with redelivery enabled) through day-by-day
//! `smelt run` windows and comparing the final table state to a single
//! full-window run. It also asserts the downstream pushdown promise
//! (`incremental_models.md` §"What the composed shape uniquely enables" —
//! "Propagation admissibility"): a partition-grain consumer of the composed
//! model (`silver.sessions`) still gets a genuine derived scan clamp against
//! it, not an unbounded full-table read — the clock propagates through the
//! composed stage instead of stopping there.
//!
//! # Porting note
//!
//! This is a smaller, focused sibling of `per_partition_equivalence.rs`
//! (same staging pattern) — scoped to `events_deduped` alone rather than the
//! full identity-resolution chain that test already covers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn datagen_bin() -> PathBuf {
    smelt_bin()
        .parent()
        .expect("smelt binary must have a parent dir")
        .join("smelt-datagen")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_owned()
}

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

/// Run `smelt-datagen` with its working directory set to `workspace` (so its
/// own relative `output: data/...` and `setup_sources.sql`'s `'data/...`
/// paths resolve without rewriting).
fn run_datagen(workspace: &Path, scale_factor: f64) {
    let datagen = datagen_bin();
    assert!(
        datagen.exists(),
        "smelt-datagen not found at {datagen:?}; run `cargo build -p smelt-datagen` first"
    );
    let out = Command::new(&datagen)
        .args(["--config", "datagen.yaml", "--scale-factor"])
        .arg(scale_factor.to_string())
        .current_dir(workspace)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt-datagen: {e}"));
    if !out.status.success() {
        panic!(
            "smelt-datagen failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// `setup_sources.sql`'s `'data/...` paths are relative to a `smelt`
/// process's own working directory (the workspace root) — but this harness
/// opens the DuckDB file directly in-process via the `duckdb` crate, whose
/// relative-path resolution is the *test binary's* cwd, not `workspace`.
/// Rewrite the `'data/` prefix to `workspace`'s own absolute `data/` dir so
/// the load works regardless of the test process's cwd.
fn setup_sources(workspace: &Path, db_path: &Path) {
    let setup_sql_path = workspace.join("setup_sources.sql");
    let sql = fs::read_to_string(&setup_sql_path)
        .unwrap_or_else(|e| panic!("read setup_sources.sql: {e}"));
    let abs_data = workspace.join("data");
    let sql = sql.replace("'data/", &format!("'{}/", abs_data.display()));

    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources.sql: {e}\nSQL:\n{sql}"));
}

fn reset_db(workspace: &Path) {
    let db = workspace.join("target/dev.duckdb");
    if db.exists() {
        fs::remove_file(&db).unwrap_or_else(|e| panic!("remove {db:?}: {e}"));
    }
}

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

/// One row of `silver.events_deduped`'s stored state, for cross-pipeline
/// comparison. Excludes `event_ts`/`amplitude_id` etc. only for brevity —
/// `event_id` plus every `MIN`-folded payload field is enough to detect any
/// divergence a redelivered duplicate could cause.
#[derive(Debug, Clone, PartialEq)]
struct DedupedRow {
    event_id: i64,
    device_id: Option<i32>,
    user_id: Option<i32>,
    first_seen_date: String,
    event_name: Option<String>,
    platform: Option<String>,
}

fn query_deduped_rows(db_path: &Path) -> Vec<DedupedRow> {
    let conn = duckdb::Connection::open(db_path)
        .unwrap_or_else(|e| panic!("open duckdb {db_path:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT event_id, device_id, user_id, first_seen_date::VARCHAR, \
                    event_name, platform \
             FROM main.silver_events_deduped ORDER BY event_id",
        )
        .unwrap_or_else(|e| panic!("prepare events_deduped query: {e}"));
    stmt.query_map([], |row| {
        Ok(DedupedRow {
            event_id: row.get(0)?,
            device_id: row.get(1)?,
            user_id: row.get(2)?,
            first_seen_date: row.get(3)?,
            event_name: row.get(4)?,
            platform: row.get(5)?,
        })
    })
    .unwrap_or_else(|e| panic!("query events_deduped rows: {e}"))
    .collect::<Result<Vec<_>, _>>()
    .unwrap_or_else(|e| panic!("collect events_deduped rows: {e}"))
}

fn stage_workspace(tmp: &Path) -> (PathBuf, PathBuf) {
    let workspace = tmp.join("workspace");
    let db_path = workspace.join("target/dev.duckdb");

    copy_dir_all(&repo_root().join("examples/web_analytics"), &workspace);
    if db_path.exists() {
        fs::remove_file(&db_path).unwrap_or_else(|e| panic!("remove pre-existing db: {e}"));
    }

    // scale_factor=0.05 (vs. per_partition_equivalence's 0.01): the ~2%
    // redelivery fraction (`datagen.yaml`'s `redelivery:` block) needs
    // enough raw rows that the 7-day window reliably contains several
    // redelivered duplicates, not zero.
    run_datagen(&workspace, 0.05);

    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    setup_sources(&workspace, &db_path);

    (workspace, db_path)
}

const DAY_WINDOWS: &[(&str, &str)] = &[
    ("2026-03-19", "2026-03-20"),
    ("2026-03-20", "2026-03-21"),
    ("2026-03-21", "2026-03-22"),
    ("2026-03-22", "2026-03-23"),
    ("2026-03-23", "2026-03-24"),
    ("2026-03-24", "2026-03-25"),
    ("2026-03-25", "2026-03-26"),
];
const START_DATE: &str = "2026-03-19";
const END_DATE_EXCLUSIVE: &str = "2026-03-26";

/// `incremental_state(events_deduped) == full_refresh(raw.events)` across a
/// day-by-day run-window decomposition of the same 7-day source snapshot —
/// the equivalence invariant (`docs/specs/incremental_models.md`
/// §"The equivalence invariant") specialised to route 3's redelivery-storm
/// absorption.
#[test]
fn test_events_deduped_redelivery_equivalence() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db_path) = stage_workspace(tmp.path());

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    smelt_run(&workspace, START_DATE, END_DATE_EXCLUSIVE, "pipeline-A");
    let rows_a = query_deduped_rows(&db_path);

    assert!(
        !rows_a.is_empty(),
        "no rows produced — check datagen + setup_sources"
    );

    // ── Pipeline B: day-by-day replay ─────────────────────────────────────
    reset_db(&workspace);
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    setup_sources(&workspace, &db_path);
    for (ws, we) in DAY_WINDOWS {
        smelt_run(&workspace, ws, we, &format!("pipeline-B [{ws}..{we})"));
    }
    let rows_b = query_deduped_rows(&db_path);

    assert_eq!(
        rows_a, rows_b,
        "day-by-day replay must equal a full-window rebuild — a redelivered \
         duplicate must fold to the same MIN-extremal state regardless of \
         which run window observed which copy"
    );

    // ── event_id is genuinely unique — the dedupe actually happened ───────
    let mut ids: Vec<i64> = rows_a.iter().map(|r| r.event_id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        n,
        "events_deduped must carry exactly one row per event_id"
    );
}

/// The plan's own admission-route claim, checked rather than trusted
/// (the phase's review checklist: "assert which route via `smelt explain`
/// in the e2e test"). `events_deduped.sql`'s doc comment and
/// `sources/raw/events.yml`'s `key_recurrence:` comment both claim route 3's
/// **declared** sub-route (`LocalitySlice::RecurrenceBounded`, checked at
/// merge time via `KeyedRecurrenceBoundViolated`) — this must fail loudly
/// if the model ever instead falls through to a *different* route (e.g. a
/// statically-derived `LocalitySlice::Window`, which would make the
/// `key_recurrence:` declaration inert and never actually exercise the
/// checked bound this test suite's redelivery-equivalence assertions above
/// are meant to be exercising).
#[test]
fn test_events_deduped_establishes_declared_recurrence_bound_route() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, _db_path) = stage_workspace(tmp.path());

    let smelt = smelt_bin();
    let out = Command::new(&smelt)
        .args(["explain", "silver.events_deduped"])
        .current_dir(&workspace)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt explain: {e}"));
    assert!(
        out.status.success(),
        "smelt explain silver.events_deduped failed (exit {:?})\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let plan_text = String::from_utf8_lossy(&out.stdout);

    assert!(
        plan_text.contains("route: route 3 (recurrence-bounded, declared key_recurrence)"),
        "silver.events_deduped must be admitted via route 3's *declared* \
         key_recurrence sub-route (the model's own doc comment's claim) — \
         got a different route entirely:\n{plan_text}"
    );
    assert!(
        plan_text.contains("slice: RecurrenceBounded"),
        "silver.events_deduped's established locality slice must be the \
         checked `RecurrenceBounded` shape, not a statically-derived, \
         unchecked `Window` — a `Window` slice here would mean the model's \
         own SQL (e.g. a self-imposed lateness WHERE filter) is short- \
         circuiting admission before the declared `key_recurrence` bound is \
         ever consulted, leaving `KeyedRecurrenceBoundViolated` untested; \
         got:\n{plan_text}"
    );
}

/// The composed shape's downstream-propagation promise
/// (`docs/specs/incremental_models.md` §"What the composed shape uniquely
/// enables" — "Propagation admissibility"): `silver.sessions`, a
/// partition-grain consumer of the composed `silver.events_deduped`, still
/// gets a genuine derived scan clamp against it — the clock propagates
/// through the composed stage rather than forcing an unbounded read.
#[test]
fn test_downstream_partition_grain_gets_pushdown_through_composed_model() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, _db_path) = stage_workspace(tmp.path());

    let smelt = smelt_bin();
    let out = Command::new(&smelt)
        .args(["explain", "silver.sessions"])
        .current_dir(&workspace)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt explain: {e}"));
    assert!(
        out.status.success(),
        "smelt explain silver.sessions failed (exit {:?})\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let plan_text = String::from_utf8_lossy(&out.stdout);

    assert!(
        !plan_text.contains("NOT partition_local"),
        "silver.sessions' read of silver.events_deduped must be partition-local \
         (a genuine derived scan clamp) — not an unbounded full-table read; \
         got:\n{plan_text}"
    );
    assert!(
        plan_text.contains("source=silver.events_deduped column=first_seen_date"),
        "silver.sessions' maintenance plan must carry a scan clamp against \
         silver.events_deduped's own first_seen_date clock — the clock must \
         propagate through the composed model, not stop at it; got:\n{plan_text}"
    );
}
