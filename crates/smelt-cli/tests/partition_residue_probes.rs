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

/// Residue: "Per-source clamp observability is partly emitted" — formerly
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences (removed by phase 6, `docs/outcomes/20260815-partition-
/// grain-residue`). LANDED: `--period` no longer gates only `--show-sql`;
/// on the whole-project `--json` path it resolves each `Bounded` source's
/// `scan_start`/`scan_end` via the same `smelt_logical::resolve_scan_window`
/// a run's pushdown filter uses.
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

    // Whole-project `--json` (no positional model name) is the mode that
    // renders `ExplainIncremental::source_bounds` via `compute_source_bounds`.
    // `--period` now reaches it (relaxed off `requires = "show_sql"` in
    // phase 6).
    let out = Command::new(smelt_bin())
        .args(["explain", "--json"])
        .args(["--period", "2026-01-01..2026-01-08"])
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

    // LANDED: given a concrete `--period 2026-01-01..2026-01-08` and the
    // 3-day lookback declared in `recent_orders`' WHERE clause, the bound
    // now carries the resolved run-relative scan window as real calendar
    // dates — `run_start − 3d` .. `run_end` (no forward margin).
    assert_eq!(
        bound.get("scan_start").and_then(|v| v.as_str()),
        Some("2025-12-29"),
        "expected a resolved scan_start in {bound:?}"
    );
    assert_eq!(
        bound.get("scan_end").and_then(|v| v.as_str()),
        Some("2026-01-08"),
        "expected a resolved scan_end in {bound:?}"
    );
}

/// Residue: "the residual open question here is a `partition_column`
/// rename, a skeleton-position change whose refusal path has no fixture or
/// diagnostic surfaced ahead of a run" —
/// `docs/specs/incremental_shapes.md` §"The partition grain" Known
/// Divergences. LANDED (phase 7, `docs/outcomes/20260815-partition-grain-
/// residue`): a rename now emits `MaintenancePartitionColumnChanged` at
/// `smelt check`, and `smelt run --full-refresh` re-addresses the table and
/// re-records the snapshot, clearing a subsequent `smelt check`.
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
         default_materialization: table\nstate:\n  mode: intervals\n",
    );
    // Two date columns are both already projected and grouped on in v1, so
    // repointing `partition_column` from one to the other in v2 changes
    // NEITHER the output columns NOR the skeleton clause — the rename would
    // be entirely invisible to `MaintenanceSkeletonChanged`/
    // `MaintenanceColumnAddNotBackfillable`, which is exactly why the
    // declared address needs its own world-fact and its own refusal
    // (`docs/outcomes/20260815-partition-grain-residue/phases/07-plan.md`
    // "Why a new code rather than `MaintenanceSkeletonChanged`").
    write_file(
        &root.join("models/seed_dates.sql"),
        "---\nmaterialization: table\n---\n\
         SELECT DATE '2026-01-01' AS event_date, DATE '2026-01-02' AS other_date\n",
    );
    let model_sql_body = "SELECT event_date, other_date, COUNT(*) AS n FROM smelt.seed_dates \
         GROUP BY event_date, other_date\n";
    let model_v1 = format!(
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n\
         ---\n\
         {model_sql_body}"
    );
    write_file(&root.join("models/renamed_mart.sql"), &model_v1);

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

    // Repoint the declared `partition_column` at the sibling column already
    // projected and grouped on — no column added/removed, no skeleton-clause
    // diff, only the declared address changed.
    let model_v2 = format!(
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: other_date\n  partition_column: other_date\n  granularity: day\n\
         ---\n\
         {model_sql_body}"
    );
    write_file(&root.join("models/renamed_mart.sql"), &model_v2);

    // `MaintenancePartitionColumnChanged` is folded into `file_diagnostics()`
    // and surfaced by the pre-execution diagnostic gate
    // (`smelt-runtime::gate::gate_diagnostics`) that `smelt run` calls before
    // compiling any model — NOT by `smelt check` (the data-test-assertion
    // runner, `smelt-cli::commands::check`, which never calls
    // `gate_diagnostics`). `smelt run` is the real refusal surface here.
    let second = Command::new(smelt_bin())
        .args(["run"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (v2): {e}"));
    let stdout = String::from_utf8_lossy(&second.stdout);
    let stderr = String::from_utf8_lossy(&second.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !second.status.success(),
        "smelt run must refuse a partition_column rename, but exited successfully.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("partition_column")
            || combined.contains("MaintenancePartitionColumnChanged"),
        "expected the refusal to name partition_column, got:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("event_date") && combined.contains("other_date"),
        "expected the refusal to name both the recorded and current column, got:\n\
         stdout: {stdout}\nstderr: {stderr}"
    );

    // The remedy leg: this is a pre-execution analyzer refusal (the
    // analyzer gate blocks unconditionally on any Error-severity
    // diagnostic — no run flag bypasses it, `docs/specs/architecture.md`
    // §"Diagnostic parity rule (analysis ↔ build)"), so the remedy is to
    // delete the model's recorded snapshot and re-run — the run then
    // addresses the table under the new column and re-records the
    // snapshot, proving it updates rather than being a dead end.
    let snapshot_path = root.join(".smelt/targets/dev/schemas/renamed_mart.json");
    assert!(
        snapshot_path.exists(),
        "expected a recorded snapshot at {snapshot_path:?}"
    );
    std::fs::remove_file(&snapshot_path).expect("remove stale snapshot");

    let third = Command::new(smelt_bin())
        .args(["run"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (post-delete): {e}"));
    assert!(
        third.status.success(),
        "smelt run must succeed once the stale snapshot is deleted.\nstderr: {}",
        String::from_utf8_lossy(&third.stderr)
    );
    let recorded: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&snapshot_path).expect("read re-recorded snapshot"),
    )
    .expect("parse re-recorded snapshot");
    assert_eq!(
        recorded["partition_column"], "other_date",
        "expected the fresh snapshot to record the new partition_column, got {recorded:?}"
    );

    let fourth = Command::new(smelt_bin())
        .args(["run"])
        .args(["--project-dir", root.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` (post-remedy): {e}"));
    assert!(
        fourth.status.success(),
        "smelt run must be clean after --full-refresh re-records the snapshot.\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&fourth.stdout),
        String::from_utf8_lossy(&fourth.stderr)
    );
}

/// Ratchet: `docs/outcomes/20260815-partition-grain-residue` closed seven pre-`docs/outcomes/`
/// partition-grain Known Divergences bullets (phases 2-7; two folded into phase 2), and the
/// 2026-09-04 decision track (`3e9c1a4a`) closed two more — non-deterministic row-set membership
/// became a permanent refusal, and per-column `data_latency` was retired in favour of
/// orchestration-only lateness — and phase 1 of `docs/outcomes/20260904-decision-residue` closed
/// the `PartitionGrainForbidsMetrics`-is-unimplemented bullet by implementing the refusal.
/// This test pins the bullet set the spec's §"The partition grain"
/// Known Divergences is allowed to carry going forward, so a future edit cannot quietly
/// reintroduce a closed residue without the change being visible here. The count ratchets DOWN
/// only: adding a lead back requires the spec to have genuinely reopened the divergence.
#[test]
fn partition_grain_residues_stay_closed() {
    let spec_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/specs/incremental_shapes.md");
    let spec = std::fs::read_to_string(&spec_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", spec_path.display()));

    let section_start = spec
        .find("### The partition grain\n")
        .expect("spec must have a \"### The partition grain\" Known Divergences section");
    let after_heading = &spec[section_start..];
    let section_end = after_heading[1..]
        .find("\n### ")
        .map(|i| i + 1)
        .unwrap_or(after_heading.len());
    let section = &after_heading[..section_end];

    let bullets: Vec<&str> = section
        .lines()
        .filter(|l| l.trim_start().starts_with("- **"))
        .collect();

    let expected_leads = [
        "Schema evolution on the partition grain is largely a definition delta now",
        "The sub-`g_part` rejection does not yet name the coarsened window",
        "`NOW()`/`CURRENT_*` are still compile-time-pinned",
    ];

    assert_eq!(
        bullets.len(),
        expected_leads.len(),
        "expected exactly {} partition-grain Known Divergences bullets (the three this outcome \
         does not own — phase 1 of `docs/outcomes/20260904-decision-residue` closed the \
         `PartitionGrainForbidsMetrics`-is-unimplemented bullet, and the decision track \
         retired `data_latency` and the row-set-membership bullet separately), found {}:\n{}",
        expected_leads.len(),
        bullets.len(),
        bullets.join("\n")
    );

    for (bullet, expected_lead) in bullets.iter().zip(expected_leads.iter()) {
        assert!(
            bullet.contains(expected_lead),
            "unexpected or reordered partition-grain Known Divergences bullet.\n\
             expected a bullet containing: {expected_lead:?}\n\
             found: {bullet:?}\n\
             (this fires either because a closed residue's bullet reappeared, or because the \
             bullet order changed — update `expected_leads` only if the reorder is intentional)"
        );
    }
}
