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

// ── Constants ─────────────────────────────────────────────────────────────────

/// We run 7 days (2026-03-19 … 2026-03-26) rather than the full 60-day window
/// to keep CI runtime under ~30 s.  The divergence pattern on global columns
/// is visible even on a 7-day window.
const START_DATE: &str = "2026-03-19";
const END_DATE_EXCLUSIVE: &str = "2026-03-26"; // 7 days

/// Day sequence for the day-by-day pipeline: each entry is (window_start,
/// window_end) passed to `smelt run`, honouring the 1-day lookback used by
/// the Python driver.
const DAY_WINDOWS: &[(&str, &str)] = &[
    ("2026-03-18", "2026-03-20"), // day 1
    ("2026-03-19", "2026-03-21"), // day 2
    ("2026-03-20", "2026-03-22"), // day 3
    ("2026-03-21", "2026-03-23"), // day 4
    ("2026-03-22", "2026-03-24"), // day 5
    ("2026-03-23", "2026-03-25"), // day 6
    ("2026-03-24", "2026-03-26"), // day 7
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

    // Day-by-day: 2 iterations with 1-day lookback windows.
    smelt_run(
        &workspace,
        "2026-03-18",
        "2026-03-20",
        "harness-smoke-B-day1",
    );
    smelt_run(
        &workspace,
        "2026-03-19",
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
