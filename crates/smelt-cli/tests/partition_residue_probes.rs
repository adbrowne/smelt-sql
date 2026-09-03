//! Characterization probes for `docs/outcomes/20260815-partition-grain-residue`,
//! phase 1 (audit) — the three residues cheapest to observe through the real
//! `smelt` binary (explain/run surfaces). See
//! `docs/outcomes/20260815-partition-grain-residue/audit.md` for the full
//! verdict table; `crates/smelt-logical/tests/partition_residue_probes.rs`
//! covers the classification-layer residues.

#![cfg(feature = "duckdb")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// Residue: "Monotone-integer `partition_column` has no end-to-end run" —
/// formerly `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences (removed by phase 5b, `docs/outcomes/20260815-partition-
/// grain-residue`). LANDED: a monotone-integer `partition_column` model now
/// runs first-run, a windowed `--batch-size`-chunked backfill, and a
/// steady-state re-run, all producing a table equal to a full-refresh
/// oracle.
///
/// Stages a partition-grain model whose `partition_column` is a plain
/// monotone `INTEGER` (`batch_id`, cast explicitly so `resolved_model_schema`
/// classifies the integer axis for real rather than falling back to the
/// axis implied by the run-window literal's form — `docs/outcomes/
/// 20260815-partition-grain-residue/phases/05a-summary.md` "For the next
/// planner"), decoupled from its (still timestamp) `event_time_column`, and
/// drives a real run through the `smelt` binary against DuckDB.
fn stage_int_partition_project(root: &Path, model_sql_extra: &str) {
    write_file(
        &root.join("smelt.yml"),
        "name: int_partition_probe\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n",
    );
    write_file(
        &root.join("models/seed_events.sql"),
        "---\n\
         materialization: table\n\
         ---\n\
         SELECT * FROM (VALUES\n\
         \x20  (1, 1, TIMESTAMP '2026-01-01 00:00:00'),\n\
         \x20  (2, 1, TIMESTAMP '2026-01-01 06:00:00'),\n\
         \x20  (3, 2, TIMESTAMP '2026-01-02 00:00:00'),\n\
         \x20  (4, 3, TIMESTAMP '2026-01-03 00:00:00')\n\
         ) AS t(id, batch_id, event_ts)\n",
    );
    write_file(
        &root.join("models/int_partition_mart.sql"),
        &format!(
            "---\n\
             materialization: table\n\
             {model_sql_extra}\
             ---\n\
             SELECT CAST(batch_id AS INTEGER) AS batch_id, event_ts, id FROM smelt.seed_events\n"
        ),
    );
}

fn query_all_rows(db_path: &Path, table: &str) -> Vec<(i64, i64)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let query = format!("SELECT batch_id, id FROM main.{table} ORDER BY batch_id, id");
    let mut stmt = conn.prepare(&query).expect("prepare");
    stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

#[test]
fn probe_integer_partition_column_run() {
    // Phased project: first run, then a windowed `--batch-size`-chunked
    // backfill, then a steady-state re-run of the same window.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("int_partition_probe");
    stage_int_partition_project(
        &root,
        "refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: batch_id\n  granularity: day\n",
    );

    let first = Command::new(smelt_bin())
        .args(["run"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (first run): {e}"));
    assert!(
        first.status.success(),
        "first run failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    // Windowed backfill: bare-integer bounds in the partition column's own
    // domain (`docs/specs/incremental_shapes.md` §"The partition grain" rule
    // 8a — "--period bounds are read in the same domain"), `--batch-size 1`
    // forcing three separate DELETE+INSERT chunks over batch_id 1, 2, 3.
    let backfill = Command::new(smelt_bin())
        .args(["run"])
        .args(["--event-time-start", "1", "--event-time-end", "4"])
        .args(["--batch-size", "1"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (backfill): {e}"));
    assert!(
        backfill.status.success(),
        "windowed backfill run failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&backfill.stdout),
        String::from_utf8_lossy(&backfill.stderr),
    );

    // Steady-state: re-running the same window must be idempotent.
    let steady_state = Command::new(smelt_bin())
        .args(["run"])
        .args(["--event-time-start", "1", "--event-time-end", "4"])
        .args(["--batch-size", "1"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (steady-state): {e}"));
    assert!(
        steady_state.status.success(),
        "steady-state re-run failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&steady_state.stdout),
        String::from_utf8_lossy(&steady_state.stderr),
    );

    let phased_rows = query_all_rows(&root.join("target/dev.duckdb"), "int_partition_mart");

    // Full-refresh oracle: the same model, materialized in one shot.
    let oracle_tmp = tempfile::TempDir::new().unwrap();
    let oracle_root = oracle_tmp.path().join("int_partition_probe_oracle");
    stage_int_partition_project(&oracle_root, "refresh: full\n");
    let oracle_build = Command::new(smelt_bin())
        .args(["build"])
        .args(["--project-dir", oracle_root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build` (oracle): {e}"));
    assert!(
        oracle_build.status.success(),
        "oracle build failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&oracle_build.stdout),
        String::from_utf8_lossy(&oracle_build.stderr),
    );
    let oracle_rows = query_all_rows(&oracle_root.join("target/dev.duckdb"), "int_partition_mart");

    assert_eq!(
        phased_rows, oracle_rows,
        "integer-axis phased run diverged from the full-refresh oracle"
    );
    assert_eq!(
        phased_rows,
        vec![(1, 1), (1, 2), (2, 3), (3, 4)],
        "unexpected row set for the integer-axis phased run"
    );
}

/// Residue: "Per-source clamp observability is partly emitted" —
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences. Actually tracked (the spec bullet's "specified ahead of a
/// tracking plan" note is stale): `docs/plans/20260704-model-updates-l4-batched.md`
/// Phase BL8, status `pending`. `compute_source_bounds`
/// (`crates/smelt-cli/src/explain.rs`) calls `derive_model_bounds` with no
/// run-window parameter at all, and `smelt explain`'s `ExplainArgs` has no
/// `--event-time-start`/`--event-time-end` flag — only `--period` gates the
/// unrelated `--show-sql` statement rendering. Inverts in phase 6.
#[test]
fn probe_explain_json_run_relative_source_bounds() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("explain_bounds_probe");
    write_file(
        &root.join("smelt.yml"),
        "name: explain_bounds_probe\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n",
    );
    // A source-declared upstream (`smelt.sources.*`) is not tracked as a
    // graph dependency at all (confirmed against `examples/timeseries`) — an
    // upstream *model* with its own `timeseries:` block is what
    // `graph.get_upstream`/`compute_source_bounds` actually populate
    // `source_bounds` from (mirrors `examples/timeseries`'s
    // `user_daily_spend` → `user_spend_rollup` pair).
    write_file(
        &root.join("models/raw_orders.sql"),
        "---\n\
         materialization: table\n\
         timeseries:\n\
         \x20 event_time_column: order_ts\n  partition_column: order_ts\n  granularity: day\n\
         ---\n\
         SELECT CAST(order_ts AS TIMESTAMP) AS order_ts, amount \
         FROM (VALUES (TIMESTAMP '2026-01-01 00:00:00', 1.0)) AS t(order_ts, amount)\n",
    );
    write_file(
        &root.join("models/recent_orders.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: order_date\n  partition_column: order_date\n  granularity: day\n\
         ---\n\
         SELECT CAST(order_ts AS DATE) AS order_date, amount \
         FROM smelt.raw_orders \
         WHERE order_ts >= CURRENT_DATE - INTERVAL '3 day'\n",
    );

    // No CLI flag exists to hand `smelt explain` a run window at all — this
    // is itself part of the residue. Whole-project `--json` (no positional
    // model name) is the mode that renders `ExplainIncremental::source_bounds`
    // via `compute_source_bounds`; there is no `--event-time-start`/`--end`
    // equivalent here, nor does the unrelated single-model `--period` flag
    // (which only gates `--show-sql` statement rendering) reach it.
    let out = Command::new(smelt_bin())
        .args(["explain", "--json"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt explain`: {e}"));
    assert!(
        out.status.success(),
        "smelt explain failed.\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let source_bounds = json["models"]["recent_orders"]["incremental"]["source_bounds"]
        .as_object()
        .unwrap_or_else(|| panic!("no models.recent_orders.incremental.source_bounds in {json}"));
    let bound = source_bounds
        .values()
        .next()
        .unwrap_or_else(|| panic!("no source bound entries in {source_bounds:?}"));

    // TODAY: the bound renders as symbolic ISO-8601 *durations*
    // (`before`/`after`), never resolved against the concrete
    // 2026-01-01..2026-01-08 window even though one was supplied via
    // `--period`. A run-relative rendering would carry concrete calendar
    // dates (`YYYY-MM-DD`) for the resolved scan start/end instead.
    let before = bound
        .get("before")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let looks_like_calendar_date =
        before.len() == 10 && before.chars().nth(4) == Some('-') && before.starts_with("202");
    assert!(
        !looks_like_calendar_date,
        "source_bounds.before ('{before}') looks like a resolved calendar date — \
         this residue is LANDED; invert this probe and update \
         docs/specs/incremental_shapes.md's Known Divergences entry"
    );
}

/// Residue: "the residual open question here is a `partition_column`
/// rename, a skeleton-position change whose refusal path has no fixture or
/// diagnostic surfaced ahead of a run" —
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences. No `docs/plans/*` tracker is cited (schema evolution is
/// otherwise a `definition_deltas.md` concern); `SkeletonPosition` and any
/// rename-specific diagnostic do not exist anywhere in the repo. Inverts in
/// phase 7.
#[test]
fn probe_partition_column_rename_refusal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("rename_probe");
    write_file(
        &root.join("smelt.yml"),
        "name: rename_probe\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n",
    );
    write_file(
        &root.join("models/seed_dates.sql"),
        "---\nmaterialization: table\n---\n\
         SELECT DATE '2026-01-01' AS event_date\n",
    );
    let model_v1 = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n\
         ---\n\
         SELECT event_date, COUNT(*) AS n FROM smelt.seed_dates GROUP BY event_date\n";
    write_file(&root.join("models/renamed_mart.sql"), model_v1);

    let first = Command::new(smelt_bin())
        .args(["run"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (build v1): {e}"));
    assert!(
        first.status.success(),
        "v1 build failed — cannot probe the rename refusal.\nstderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    // Rename the skeleton-position field: `partition_column` from
    // `event_date` to a differently-named (but equally valid) column.
    let model_v2 = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_day\n  partition_column: event_day\n  granularity: day\n\
         ---\n\
         SELECT event_date AS event_day, COUNT(*) AS n FROM smelt.seed_dates GROUP BY event_date\n";
    write_file(&root.join("models/renamed_mart.sql"), model_v2);

    let second = Command::new(smelt_bin())
        .args(["check"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt check` (v2): {e}"));
    let stdout = String::from_utf8_lossy(&second.stdout);
    let stderr = String::from_utf8_lossy(&second.stderr);

    // TODAY: no named diagnostic exists for a `partition_column` rename —
    // `smelt check` has nothing to say about it (passes clean), so the
    // refusal path this bullet asks for has no fixture. If a future fix adds
    // one, this probe should see a non-zero exit / a diagnostic mentioning
    // `partition_column` and be inverted.
    let mentions_partition_rename =
        stdout.contains("partition_column") || stderr.contains("partition_column");
    assert!(
        !mentions_partition_rename,
        "smelt check already surfaces a partition_column-rename diagnostic — \
         this residue is LANDED; invert this probe and update \
         docs/specs/incremental_shapes.md's Known Divergences entry.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
}
