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

/// Gap 2 — bare-integer run-window bounds are only axis-resolved on the
/// dry-run path.
///
/// `smelt run --event-time-start/--event-time-end` with bare-integer
/// bounds (the integer partition axis) is meant to sidestep gap 1 — no
/// per-dialect literal typing needed for a bare integer. It does, on the
/// `--dry-run` path (`execute/window.rs::parse_run_window_in_axis`,
/// resolving each selected model's own axis correctly: the emitted
/// `DELETE`/`INSERT` region text carries a bare, unquoted integer literal,
/// confirmed by direct inspection). The **real** (non-dry-run) execution
/// path does not: `execute/window.rs::parse_run_window` returns `(None,
/// None)` for a bare-integer pair (by design — it only serves the
/// calendar-only consumers of the global `start_date`/`end_date`), and the
/// `(None, None)` branch of `execute/project/mod.rs`'s dispatch match
/// builds the model's `TimeRange` with a hardcoded calendar axis rather
/// than re-resolving the model's own axis the way `parse_run_window_in_
/// axis` does for `build_model_plans`. The result: a genuine integer-axis
/// model's real run injects `batch_id >= '1' AND batch_id < '2'` — a
/// quoted string against an `INTEGER` column — which DuckDB accepts
/// (implicit cast) and Trino refuses (`Cannot apply operator: integer <=
/// varchar(1)`, measured live). This is a real execution bug, not a Trino
/// gap; DuckDB's leniency has been masking it. `crates/smelt-cli/tests/
/// partition_residue_probes.rs::probe_integer_partition_column_run`
/// exercises the identical shape today only against DuckDB, so it has
/// never caught this.
#[allow(dead_code)]
fn gap_2_real_run_does_not_reresolve_integer_axis() {}

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
