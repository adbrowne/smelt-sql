#![cfg(feature = "duckdb")]
//! Phase B3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
//! `smelt run --since-upstream` end to end through `examples/web_analytics`'s
//! flagship composed model, `silver.events_deduped` (`grain: key` +
//! `timeseries:`, admitted via key temporal locality's route 3 —
//! recurrence-bounded, declared `sources.raw.events`'s
//! `mutation_profile.key_recurrence`). A landed delta declared on the raw
//! source propagates *through* the composed node (widened by its recurrence
//! bound, `incremental_shapes.md` §"What the composed shape uniquely
//! enables") to its downstream consumer `silver.sessions` — the composed
//! stage sits inside a real propagation chain end to end, not just at its
//! own creation edge or mid-chain in a synthetic fixture (the same real
//! workspace `events_deduped_redelivery_equivalence.rs` stages, reused
//! here for the propagation leg instead of the redelivery-equivalence leg).

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

fn stage_workspace(tmp: &Path) -> (PathBuf, PathBuf) {
    let workspace = tmp.join("workspace");
    let db_path = workspace.join("target/dev.duckdb");

    copy_dir_all(&repo_root().join("examples/web_analytics"), &workspace);
    if db_path.exists() {
        fs::remove_file(&db_path).unwrap_or_else(|e| panic!("remove pre-existing db: {e}"));
    }

    // `--since-upstream` builds the propagation graph over EVERY discovered
    // model unconditionally (no `--select` scoping), so any pre-existing,
    // out-of-scope graph limitation ANYWHERE in the workspace refuses the
    // whole run — this plan's own "Explicitly deferred" section names both
    // limitations this real fixture happens to carry:
    //
    //   - `silver.sessions_chained` self-references its own prior output (an
    //     intentional recursive-accumulation demo) — "time-unrolled
    //     self-edges in the graph" are refused fail-loud
    //     (`MaintenanceGraphUnsupportedNode`, "self-referential").
    //     `silver.events_enriched` is the only other model that reads it.
    //   - `silver.device_user_edges` is a **bare** keyed model (`grain: key`,
    //     no `timeseries:` — unlike the locality-admitted composed
    //     `events_deduped`) — "keyed dirt-sets... bare keyed nodes still
    //     refuse" (`refuse_keyed_nodes`, S12) fail-loud refuses ANY edge
    //     touching it, including its own inbound edge from `events_deduped`,
    //     even though nothing downstream of it is on this test's own
    //     propagated path. Its only readers are `gold/*`.
    //
    // Excluding both leaves the `bronze.raw_events -> silver.events_parsed /
    // events_deduped -> silver.sessions` chain this test exercises
    // untouched — the composed node still sits inside a real multi-hop
    // propagation chain, just not all the way through to the (unrelated,
    // deferred-limitation) identity/marts layer.
    for rel in [
        "models/silver/sessions_chained.sql",
        "models/silver/events_enriched.sql",
        "models/silver/device_user_edges.sql",
        "models/gold/eventstream_with_identity.sql",
        "models/gold/identity_backward_fill.sql",
        "models/gold/identity_connected_components.sql",
        "models/gold/identity_forward_only.sql",
        "models/marts/daily_active_users_by_method.sql",
        "models/marts/identity_method_comparison.sql",
    ] {
        fs::remove_file(workspace.join(rel)).unwrap_or_else(|e| panic!("remove {rel}: {e}"));
    }

    // Same scale factor as `events_deduped_redelivery_equivalence.rs` — the
    // dataset spans 2026-03-19..2026-03-26 (7 days), the same window this
    // test's `--landed` interval is chosen against.
    run_datagen(&workspace, 0.05);

    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");
    setup_sources(&workspace, &db_path);

    (workspace, db_path)
}

fn run_since_upstream(workspace: &Path, args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("run")
        .arg("--since-upstream")
        .args(args)
        .current_dir(workspace)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run --since-upstream`: {e}"))
}

fn deduped_first_seen_dates(db_path: &Path) -> Vec<String> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT first_seen_date::VARCHAR FROM main.silver_events_deduped \
             ORDER BY 1",
        )
        .expect("prepare");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

fn sessions_row_count(db_path: &Path) -> i64 {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.query_row("SELECT COUNT(*) FROM main.silver_sessions", [], |row| {
        row.get(0)
    })
    .expect("count sessions")
}

/// A one-day landed delta on `sources.raw.events` propagates *through* the
/// composed `silver.events_deduped` node (widened by its declared route-3
/// recurrence bound — 3 days, `sources/raw/events.yml`'s
/// `key_recurrence.window` — the SAME derivation `smelt explain
/// silver.events_deduped` reports) to `silver.sessions`. Asserts the printed
/// dirty set names both edges, `events_deduped`'s own written region is
/// exactly the widened window (not the whole table), and `sessions`
/// materializes real propagated rows. A second, delta-less invocation is
/// then a byte-identical no-op.
#[test]
fn landed_delta_on_raw_events_propagates_through_composed_node_to_sessions() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db_path) = stage_workspace(tmp.path());

    // The dataset's last day is 2026-03-25 (7 days from 2026-03-19);
    // `events_deduped`'s route-3 `KeyLocality::RecurrenceBounded` derives
    // `margin_before = 3d` (the declared `key_recurrence.window` itself) and
    // `margin_after = 0d` (no derived self-scan bound) — reflected FORWARD
    // through `DayInterval::reflect`'s swapped convention
    // (`start: delta.start - after_days, end: delta.end + before_days`,
    // `crates/smelt-logical/src/maintenance/propagate.rs`), a one-day delta
    // landed on 2026-03-22 dirties `events_deduped`'s own output axis over
    // `[2026-03-22, 2026-03-22 + 1 + 3) = [2026-03-22, 2026-03-26)` —
    // clipped to the dataset's real last day, `2026-03-25`.
    let output = run_since_upstream(
        &workspace,
        &[
            "--source",
            "sources.raw.events",
            "--landed",
            "2026-03-22..2026-03-23",
        ],
    );
    assert!(
        output.status.success(),
        "since-upstream run through the composed node must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dirty set (--since-upstream):"),
        "must print the dirty set before acting: {stdout}"
    );
    assert!(
        stdout.contains("silver.events_deduped <- raw.events"),
        "the dirty set must show the source -> composed-node edge: {stdout}"
    );
    assert!(
        stdout.contains("silver.sessions <- silver.events_deduped"),
        "the dirty set must show the composed-node -> downstream edge, proving \
         the composed stage sits inside the propagation chain: {stdout}"
    );

    let expected_dates = vec![
        "2026-03-22".to_string(),
        "2026-03-23".to_string(),
        "2026-03-24".to_string(),
        "2026-03-25".to_string(),
    ];
    assert_eq!(
        deduped_first_seen_dates(&db_path),
        expected_dates,
        "events_deduped must materialize exactly the projected (widened-by-\
         recurrence-bound) region — nothing outside it"
    );

    let sessions_count_after_first_run = sessions_row_count(&db_path);
    assert!(
        sessions_count_after_first_run > 0,
        "the composed node's downstream consumer must materialize real \
         propagated rows, not stay empty"
    );

    // A second, delta-less invocation must be a byte-identical no-op — no
    // implicit whole-table or recorded-state fallback.
    let output2 = run_since_upstream(&workspace, &[]);
    assert!(
        output2.status.success(),
        "a delta-less --since-upstream invocation must still succeed: stderr={}",
        String::from_utf8_lossy(&output2.stderr)
    );
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(
        stderr2.contains("propagated nothing"),
        "a delta-less invocation must report it ran nothing: stdout={stdout2} stderr={stderr2}"
    );

    assert_eq!(
        deduped_first_seen_dates(&db_path),
        expected_dates,
        "a delta-less invocation must not change events_deduped's materialized state"
    );
    assert_eq!(
        sessions_row_count(&db_path),
        sessions_count_after_first_run,
        "a delta-less invocation must not change sessions' materialized state"
    );
}
