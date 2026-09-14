# Phase 3a — the real run resolves each model's window in its own partition axis

## Objective

On the real (non-`--dry-run`) execution path the batch `TimeRange` is constructed with a hardcoded
`PartitionAxis::Calendar`, so an integer-axis model's injected scan-window and output-clamp
predicates render quoted (`batch_id >= '1'`) instead of bare. DuckDB, Spark and BigQuery coerce it;
Trino refuses (`Cannot apply operator: integer <= varchar(1)`, measured live in phase 3). Fixing it
unblocks every live `execute_project` proof this outcome owes — criterion 2 (the reachable families
execute end-to-end) and criterion 7 (the generative gate runs through the real pipeline).

This is gap 2 of phase 3's block report. It is not Trino-specific: the quoted literal is wrong on
every engine, and the other three merely tolerate it.

## Spec delta

None. `docs/specs/incremental_shapes.md` §"The partition grain" rule 8a already specifies that
`--event-time-start`/`--event-time-end` bounds are read in the partition column's own domain, and
`docs/specs/timeseries.md` §"Validation rules" rule 9 already specifies axis resolution from the
output schema. The dry-run path already obeys both (`parse_run_window_in_axis`); this phase makes
the real path match the spec it already has. No Known Divergences entry exists to delete.

## Tests

Red-green, in this order:

1. `crates/smelt-runtime/tests/partition_axis_windowing.rs::
   real_run_batch_sql_renders_integer_axis_bounds_bare` — drive a `refresh: incremental`,
   `grain: partition`, integer `partition_column` model through `execute_project` against DuckDB
   with `--event-time-start 1 --event-time-end 4`, capturing `reporter.model_compiled`; assert the
   batch-filtered SQL contains a bare `>= 1` / `< 4` bound on the partition column and contains no
   `'1'`/`'4'` quoted form. Red today (the hardcoded `Calendar` axis quotes both bounds).
2. `crates/smelt-runtime/tests/partition_axis_windowing.rs::
   real_run_batch_sql_still_quotes_calendar_axis_bounds` — the same shape on a `DATE`
   partition column still renders the calendar literal, so the fix does not invert the axis for the
   overwhelmingly common case. (Green before and after; a regression fence, not a red test.)
3. `crates/smelt-cli/tests/partition_residue_probes.rs::probe_integer_partition_column_run` —
   extend the existing probe (which today only asserts the runs succeed against DuckDB) with a
   row-level assertion that the `--batch-size 1` backfill wrote exactly batches 1, 2 and 3 and the
   steady-state re-run left them unchanged. Guards against the fix changing *which* rows an
   integer-axis batch covers, not just how its bound is spelled.
4. `crates/smelt-cli/tests/trino_incremental_families.rs::
   integer_axis_incremental_model_runs_on_trino` — the live leg: the same integer-axis model run
   through the CLI against the live Trino/Iceberg tier, asserting success and the expected rows.
   Replaces the `#[allow(dead_code)] fn gap_2_...()` documentation anchor with a real test.
   **Emit `<<PHASE_BLOCKED>>` if `scripts/trino-env.sh` cannot reach the coordinator — never skip
   green.**

## Tasks

1. Read `crates/smelt-runtime/src/execute/project/mod.rs` and enumerate every construction of a
   `TimeRange`/`PartitionRange` on the real execution path that hardcodes `Calendar` — at least
   lines ~1727, ~2191, ~2359, ~3376, ~3391 (`smelt_logical::PartitionAxis::Calendar` and
   `smelt_backend::PartitionAxis::Calendar`). Record the list in the summary.
2. Write tests 1 and 2; confirm test 1 fails with the quoted bound in the message.
3. Thread the model's resolved axis to each of those sites. The axis is already available: it is
   `partition_axes` (built at mod.rs:267 via `resolve_partition_axes`, already passed to
   `build_model_plans`) and it is implied by the batch's own `PartitionPoint` variant. Prefer the
   resolved-axis map, falling back to the `PartitionPoint`'s own variant, so no site re-derives the
   axis from literal text. Do NOT introduce a second axis-resolution site (criterion 6's shape).
4. Where a `(Some(s), Some(e))` calendar-`NaiveDate` arm is genuinely calendar-only by
   construction, leave `Calendar` and say so in a one-line comment naming why, so a later reader
   does not have to re-derive which of the five sites were real bugs.
5. Run test 3's extended probe; fix any row-coverage fallout.
6. Bring up the live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`) and land
   test 4, replacing the gap-2 doc anchor in `trino_incremental_families.rs` (leave the gap-1 and
   gap-3 anchors and their doc comments intact — they are still open).
7. Write `phases/3a-summary.md` naming the sites changed, the sites deliberately left `Calendar`,
   and whether gap 2 also affected the delete-and-insert `PartitionRange` that phase 4 will build on.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test partition_axis_windowing --test windowing_parity --test
  partition_grid_validation --quiet`
- `cargo test -p smelt-cli --test partition_residue_probes --quiet`
- `cargo test -p smelt-runtime --test execute_parity --test dry_run_statements --quiet`
  (CLI/UI pipeline parity, and dry-run must still render identically to the real path)
- Live tier: `cargo test -p smelt-cli --test trino_incremental_families --quiet` and
  `cargo test -p smelt-backend-trino --test backend_live --quiet` (phase 3's three proofs must
  still pass unchanged)

## Commit message

`fix(runtime): resolve the real run's batch TimeRange in each model's own partition axis`
