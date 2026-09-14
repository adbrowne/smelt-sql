//! `20260913-trino-incremental` phase 3's two ledger-free families —
//! insert-only append and the whole-row `MERGE` upsert — proved at the
//! `Backend`-trait/emitter level rather than through a full CLI
//! `execute_project` invocation. See this file's doc comments below for
//! why: three separate, pre-existing, cross-cutting bugs (none introduced
//! by, or specific to, `MaintenanceDialect::Trino`) were found live and
//! block a real `smelt run`/`smelt rebuild` end-to-end for these families
//! today, on any backend — they are simply masked everywhere else by
//! DuckDB's/Spark's/BigQuery's more lenient implicit literal coercion.
//! Trino's strictness is what surfaced them.
//!
//! What phase 3 DID prove live, unaffected by these three gaps:
//! - `crates/smelt-backend-trino/tests/backend_live.rs::
//!   insert_into_from_query_appends_and_leaves_prior_rows_intact`
//! - `crates/smelt-backend-trino/tests/backend_live.rs::
//!   delete_and_insert_transactional_covers_two_disjoint_windows`
//! - `crates/smelt-backend-trino/tests/backend_live.rs::
//!   merge_into_upserts_matched_and_unmatched_rows_across_two_runs`
//!
//! Each drives the real `Backend` trait method a live `execute_project` run
//! would call, over the real `smelt_logical::maintenance::emit` statement
//! text `MaintenanceDialect::Trino` renders — the same emitters, the same
//! statement-emission single-owner path, just invoked directly instead of
//! through the CLI's window-string parsing and cell-derivation layers,
//! which is exactly what the three gaps below live in.
//!
//! Gap 2 was landed in phase 3a (`docs/outcomes/20260913-trino-incremental/
//! phases/03a-plan.md`) — `integer_axis_incremental_model_runs_on_trino`
//! below is the real CLI-level proof, replacing what was a documentation-only
//! anchor function. Gaps 1 and 3 are still open.

mod common;
use common::{drop_trino_schema, fetch_trino_rows, trino_env, trino_schema, trino_target_block};

use std::fs;
use std::path::Path;
use std::process::Command;

/// Gap 1 — the calendar-literal type-coercion gap.
///
/// `smelt_logical::maintenance::emit::types::partition_literal`'s calendar
/// axis renders a bare quoted string (`'2026-01-01'`), and
/// `smelt-runtime`'s injected scan-window predicate
/// (`transformer.rs::inject_time_filter`/`inject_source_filters`) does the
/// same. DuckDB, Spark and BigQuery all implicitly coerce that string
/// against a `DATE`/`TIMESTAMP` column in a comparison; Trino refuses
/// outright: `Cannot apply operator: date <= varchar(10)` (measured live,
/// a plain `grain: partition` passthrough model over a `DATE`-partitioned
/// append-only source). Fixing this needs a dialect-aware literal — an
/// ANSI `DATE '...'`/`TIMESTAMP '...'` spelling works on all four engines,
/// but changing `partition_literal`'s output format is a global,
/// byte-format change that ~19 files across the workspace pin exact
/// literal text against, so it needs its own reviewed change, not a
/// phase-3-scoped one.
#[allow(dead_code)]
fn gap_1_calendar_literal_type_coercion() {}

fn stage_int_partition_project(tmp: &tempfile::TempDir, schema: &str) -> std::path::PathBuf {
    let root = tmp.path().join("int_partition_trino");
    fs::create_dir_all(root.join("models")).unwrap();

    let yml = format!(
        "name: int_partition_trino\nversion: 1\npaths:\n  - models\ntargets:\n{}default_materialization: table\n",
        trino_target_block(schema)
    );
    fs::write(root.join("smelt.yml"), yml).unwrap();

    fs::write(
        root.join("models/seed_events.sql"),
        "---\n\
         materialization: table\n\
         ---\n\
         SELECT * FROM (VALUES\n\
         \x20  (CAST(1 AS BIGINT), CAST(1 AS BIGINT), TIMESTAMP '2026-01-01 00:00:00'),\n\
         \x20  (CAST(2 AS BIGINT), CAST(1 AS BIGINT), TIMESTAMP '2026-01-01 06:00:00'),\n\
         \x20  (CAST(3 AS BIGINT), CAST(2 AS BIGINT), TIMESTAMP '2026-01-02 00:00:00'),\n\
         \x20  (CAST(4 AS BIGINT), CAST(3 AS BIGINT), TIMESTAMP '2026-01-03 00:00:00')\n\
         ) AS t(id, batch_id, event_ts)\n",
    )
    .unwrap();
    fs::write(
        root.join("models/int_partition_mart.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: batch_id\n  granularity: day\n\
         ---\n\
         SELECT CAST(batch_id AS BIGINT) AS batch_id, event_ts, id FROM smelt.seed_events\n",
    )
    .unwrap();

    root
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .args(args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

/// Gap 2 (landed) — the live leg: an integer-`partition_column` model runs a
/// `--batch-size 1` backfill and a steady-state re-run end to end through
/// `smelt run --target trino`, no longer refused with `Cannot apply
/// operator: integer <= varchar(1)` now that the real execution path
/// resolves each batch's `TimeRange` in the model's own partition axis
/// (`docs/outcomes/20260913-trino-incremental/phases/03a-plan.md`).
#[test]
fn integer_axis_incremental_model_runs_on_trino() {
    let Some(_env) = trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping integer_axis_incremental_model_runs_on_trino");
        return;
    };
    let schema = trino_schema("int_partition");

    let tmp = tempfile::TempDir::new().unwrap();
    let root = stage_int_partition_project(&tmp, &schema);

    let first = run_smelt(&root, &["--target", "trino"]);
    assert!(
        first.status.success(),
        "first run (seed) failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let backfill = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "4",
            "--batch-size",
            "1",
        ],
    );
    assert!(
        backfill.status.success(),
        "windowed backfill failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&backfill.stdout),
        String::from_utf8_lossy(&backfill.stderr),
    );

    let steady_state = run_smelt(
        &root,
        &[
            "--target",
            "trino",
            "--event-time-start",
            "1",
            "--event-time-end",
            "4",
            "--batch-size",
            "1",
        ],
    );
    assert!(
        steady_state.status.success(),
        "steady-state re-run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&steady_state.stdout),
        String::from_utf8_lossy(&steady_state.stderr),
    );

    let mut rows = fetch_trino_rows(&schema, "int_partition_mart")
        .into_iter()
        .map(|r| (r[0].clone(), r[2].clone()))
        .collect::<Vec<_>>();
    rows.sort();
    let expected = vec![
        ("1".to_string(), "1".to_string()),
        ("1".to_string(), "2".to_string()),
        ("2".to_string(), "3".to_string()),
        ("3".to_string(), "4".to_string()),
    ];
    assert_eq!(rows, expected, "unexpected row set after the phased run");

    drop_trino_schema(&schema);
}

/// Gap 3 — a snapshot-reconcile-shaped keyed model's `ColumnScopedMerge`
/// downgrade assumes a `ScanClamp` that does not exist for it.
///
/// The natural model shape for "the whole-row MERGE upsert" — `refresh:
/// incremental`, `grain: key`, a plain `ANY_VALUE`-aggregate passthrough of
/// an **unclocked** `mutable_snapshot` source, no upstream model edge —
/// does not dispatch through `smelt-runtime/src/cumulative.rs`'s
/// `execute_snapshot_reconcile` (as its own doc comments and
/// `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs`'s ledger-
/// absence test might suggest). It dispatches through the general
/// `smelt_logical::maintenance::choice` cell-derivation system instead,
/// whose *ideal* technique for this cell is `ColumnScopedMerge` (confirmed
/// via `smelt explain --json`'s `state_downgrade.original`). On a fully
/// degraded dialect (`realisable_state_structures` empty — Trino and Spark
/// alike), `ColumnScopedMerge` downgrades to `PerGroupRecompute`, and that
/// downgrade route assumes a derivable `ScanClamp` — sound for a *clocked*,
/// windowed `PerGroupRecompute` cell, but this cell's trigger is
/// `UpstreamMutation`, not a clock, so no `ScanClamp` exists to derive.
/// Execution refuses with `MaintenanceRepairSliceMissing` rather than
/// falling back to the full-scan recompute the `key_scope: None`
/// "reachable" row in `docs/specs/multi_backend.md`'s landing table already
/// promises. `20260913-trino-ledger`'s Spark twin realises the identical
/// fully-degraded posture, so this is very likely reachable on Spark too
/// for the same model shape — not Trino-specific, and it sits in
/// `smelt_logical::maintenance::choice`'s cell-derivation layer, not in
/// this phase's (`MaintenanceDialect::Trino`) emitters.
#[allow(dead_code)]
fn gap_3_column_scoped_merge_downgrade_needs_no_scan_clamp_route() {}
