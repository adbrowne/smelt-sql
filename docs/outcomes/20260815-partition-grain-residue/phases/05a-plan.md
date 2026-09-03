# Phase 5a — Partition-axis domain: a typed run window that admits monotone integers

**Row:** 5a · **Success criteria advanced:** 5 (partially — chunking + run-window arithmetic;
scan-filter/DELETE emission and the end-to-end run land in 5b)

## Objective

Today the run window is `chrono::NaiveDate` end to end, so a partition-grain model whose
`partition_column` is a monotone integer emits date literals against an `INT32` column and dies at
the DELETE (confirmed: `DELETE FROM main.int_partition_mart WHERE batch_id >= '2026-01-01' …` →
`Could not convert string '2026-01-01' to INT32`). This phase makes the *partition axis* a typed,
two-domain value — calendar or unit-step integer — and generalizes run-window parsing, validation,
alignment and backfill chunking over it. Emission and the e2e proof are phase 5b.

## Spec delta (spec-first — the implement step makes these edits before code)

1. `docs/specs/timeseries.md` §Semantics — new rule under the validation rules, **"Partition axis
   domain"**: the partition axis is calendar when `partition_column` resolves to a date/timestamp
   type and a **unit-step integer grid** when it resolves to an integer type (one partition = one
   integer value). On an integer axis `granularity` remains the declared propagation grain
   (grain-alignment, graph edges — unchanged) but is *not* the chunk step; the chunk step is one
   unit. Amend §"Run window alignment": granularity-boundary alignment is a calendar-axis rule; on
   an integer axis the only requirement is `end > start`.
2. `docs/specs/incremental_shapes.md` §"The partition grain" Rules — the run window's bounds are
   supplied in the axis's own domain (`YYYY-MM-DD` for calendar, a bare integer for an integer
   axis); a bound whose form contradicts the resolved `partition_column` type is a hard refusal.
   `--batch-size N` counts N partition units on an integer axis. Day-typed widening inputs
   (`columns.<c>.data_latency`, a seconds-domain lookback/lookahead or partition skew) have no
   conversion into an integer axis and are refused fail-closed, never coerced to units.
   Leave the Known Divergences bullet in place (5b removes it).

## Tests (red-green)

- `smelt-logical` (`analysis/`, unit): `partition_axis_for_type_classifies_date_integer_and_other`
  — Date/Timestamp → `Calendar`, SmallInt/Integer/BigInt → `Integer`, Text/Unknown → `None`.
- `smelt-runtime::windowing`: `partition_point_display_and_sql_literal` — `Date` renders
  `2026-01-01` / `'2026-01-01'`; `Integer` renders `7` / `7` (unquoted).
- `integer_axis_chunks_by_unit_steps` — `[1, 4)`, per-partition → `[1,2) [2,3) [3,4)`.
- `integer_axis_batch_size_counts_units` — `[1, 6)`, `batch_size = 2` → `[1,3) [3,5) [5,6)`.
- `integer_axis_run_window_requires_positive_span_only` —
  `validate_run_window_against_partition_grid` accepts `[3, 4)` on an integer axis (no
  granularity-boundary or `g_part` comparison), rejects `[4, 4)`.
- `integer_axis_refuses_date_bounds` / `calendar_axis_refuses_integer_bounds` — a bound whose form
  contradicts the resolved type is `Err`, naming the column and both domains.
- `integer_axis_refuses_day_typed_widening` — non-zero `data_latency_days` (and, separately, a
  seconds-domain lookback) on an integer axis is `Err`, never silently zeroed or unit-coerced.
- `smelt-runtime::execute`: `parse_run_window_accepts_integer_bounds` — `--event-time-start 1
  --event-time-end 4` parses to two `PartitionPoint::Integer`s.
- Regression guard (must stay green untouched): `cargo test -p smelt-cli --test rebuild_dry_run`
  and `-p smelt-runtime --test statement_parity` — every date-axis chunk decomposition and every
  emitted literal is byte-identical to today.

## Tasks

1. Make the two spec edits above.
2. Add `PartitionAxis { Calendar, Integer }` + pure `partition_axis_for_type(&DataType) ->
   Option<PartitionAxis>` in `smelt-logical` (next to `analysis/monotonicity.rs`, which already
   carries `Offset::Integer`), re-exported from the crate root.
3. Add `PartitionPoint { Date(NaiveDate), Integer(i64) }` to
   `crates/smelt-runtime/src/windowing.rs`: `Display`, `sql_literal()` (quoted vs bare),
   `parse_in_axis(&str, PartitionAxis)`, and the axis arithmetic the chunker needs
   (`next_partition_start`, `advance_units`, `units_between`).
4. Retype `IncrementalBatch`'s four fields to `PartitionPoint`; thread a `PartitionAxis` argument
   through `compute_incremental_windows{,_ordered}` and `compute_incremental_windows_impl`,
   branching the existing calendar arithmetic vs unit stepping.
5. Generalize `validate_run_window_alignment` / `validate_run_window_against_partition_grid` over
   the axis: integer axis ⇒ positive-span check only, `derive_partition_grid_unit` not consulted.
6. Add the fail-loud refusals (domain mismatch; day-typed widening on an integer axis) as `Err`
   returns from the windowing entry points, each naming the model and the offending input — no
   coercion, no silent zeroing.
7. In `execute.rs`, resolve each selected model's `partition_column` type into a
   `HashMap<String, DataType>` before `build_model_plans` (reuse the `smelt_db::
   resolved_model_schema` read `UpstreamSchemas::from_database` already performs — share/hoist it,
   do not build `UpstreamSchemas` twice) and pass the resolved axis in. Resolved type decides the
   axis; when the type is unresolvable, fall back to the axis implied by the run-window literal
   form with a `tracing::warn!` (an undecidable type is not a positive disproof — same fail-open
   posture as `derive_partition_grid_unit`), and generalize `parse_run_window` accordingly.
8. Mechanically replace the ~32 `batch.partition_*.format("%Y-%m-%d")` / `filter_*` sites in
   `execute.rs` with `PartitionPoint`'s `Display` — no change to literal *quoting* in this phase
   (that is 5b's `sql_literal()` work; date-axis output must stay byte-identical).
9. Update the `smelt-cli` callers that construct or validate windows (`temporal.rs`,
   `commands/explain.rs`, `commands/run.rs`, `helpers.rs`) to pass/propagate the axis.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity` and `-p smelt-cli --test rebuild_dry_run`
  — the date-axis byte-identity guard.
- `cargo test -p smelt-cli --test maintenance_conformance` — the equivalence gate is unchanged.
- `cargo test -p smelt-cli --test partition_residue_probes probe_integer_partition_column_run` —
  still red (still a residue; it inverts in 5b), and its failure must have *moved* off the
  DELETE-literal conversion error.

## Commit message

`feat(runtime): typed partition axis — run windows and backfill chunking over monotone integers`
