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
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences. Tracked: `docs/plans/20260704-model-updates-l4-batched.md`
/// (Phase BL6 landed only the trace/bound-derivation admission in
/// `crates/smelt-logical/src/analysis/monotonicity.rs`; run windows,
/// backfill chunking, and scan-filter injection stay date-typed throughout
/// `crates/smelt-runtime/src/windowing.rs`'s `IncrementalWindows` —
/// `partition_start`/`partition_end`/`filter_start`/`filter_end` are all
/// `chrono::NaiveDate`, and the CLI's own `--event-time-start`/
/// `--event-time-end` flags are documented `YYYY-MM-DD` only). Inverts in
/// phase 5.
///
/// Stages a partition-grain model whose `partition_column` is a plain
/// monotone `INTEGER` (`batch_id`), decoupled from its (still timestamp)
/// `event_time_column`, and drives a real windowed run through the `smelt`
/// binary against DuckDB. Pins wherever it first breaks today.
#[test]
fn probe_integer_partition_column_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("int_partition_probe");
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
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: batch_id\n  granularity: day\n\
         ---\n\
         SELECT batch_id, event_ts, id FROM smelt.seed_events\n",
    );

    // First run: no window — a plain first-build. If this fails, the residue
    // blocks even before any windowed arithmetic runs.
    let first = Command::new(smelt_bin())
        .args(["run"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (first run): {e}"));

    // Windowed run: forces the DELETE+INSERT partition-clamp arithmetic that
    // `windowing.rs` computes in `chrono::NaiveDate` space.
    let windowed = Command::new(smelt_bin())
        .args(["run"])
        .args(["--event-time-start", "2026-01-01"])
        .args(["--event-time-end", "2026-01-04"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (windowed): {e}"));

    // TODAY: at least one of these fails — the residue's whole premise is
    // that no end-to-end run exists for a monotone-integer partition_column.
    // If both succeed, the residue is LANDED; invert this probe (assert both
    // succeed and the output matches a full-refresh oracle) and update
    // docs/specs/incremental_shapes.md's Known Divergences entry.
    assert!(
        !first.status.success() || !windowed.status.success(),
        "both first-run and windowed run succeeded for a monotone-integer \
         partition_column model — this residue is LANDED.\n\
         first stdout: {}\nfirst stderr: {}\n\
         windowed stdout: {}\nwindowed stderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        String::from_utf8_lossy(&windowed.stdout),
        String::from_utf8_lossy(&windowed.stderr),
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
