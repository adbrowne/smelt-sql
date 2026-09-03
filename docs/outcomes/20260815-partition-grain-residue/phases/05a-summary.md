# Phase 5a summary — typed partition axis (run windows + backfill chunking)

## Shipped

- `PartitionAxis { Calendar, Integer }` + `partition_axis_for_type(&DataType)` — `crates/smelt-logical/src/analysis/partition_axis.rs`, re-exported from `smelt-logical`'s crate root.
- `PartitionPoint { Date(NaiveDate), Integer(i64) }` (`Display`, `sql_literal()`, `parse_in_axis`, `next_partition_start`, `advance_units`, `units_between`) in `crates/smelt-runtime/src/windowing.rs`.
- `IncrementalBatch`'s four fields retyped to `PartitionPoint`; `compute_incremental_windows{,_ordered}`/`_impl` take a `PartitionAxis` and dispatch to a new `compute_calendar_windows` (unchanged math, `PartitionPoint::Date`-wrapped) vs `compute_integer_windows` (unit-step chunker, fail-closed day-typed-widening refusal).
- `validate_run_window_against_partition_grid` generalized to `PartitionPoint` args, dispatching on the point's own variant (no separate axis param needed); integer axis does only the `end > start` check.
- `crates/smelt-runtime/src/execute.rs`: `resolve_partition_axes` (reuses `smelt_db::resolved_model_schema`, the same read `UpstreamSchemas::from_database` performs — no second `UpstreamSchemas`), `parse_run_window_in_axis` (generalizes `parse_run_window` per-model), `window_for_axis` dispatcher; `build_model_plans` restructured to resolve axis + window per model before dispatching; the ~20 `batch.partition_*.format("%Y-%m-%d")` sites replaced with `PartitionPoint`'s `Display` (no quoting change, byte-identical calendar output).
- `smelt-cli`: `temporal.rs`/`lib.rs` re-export `PartitionAxis`/`PartitionPoint`; `explain.rs` passes `PartitionAxis::Calendar` explicitly (axis-aware `explain` is future work, noted inline).
- Spec edits: `docs/specs/timeseries.md` new validation rule 9 ("Partition axis domain") + amended "Granularity arithmetic"; `docs/specs/incremental_shapes.md` new rule 8a under "Partition-grain constraints".
- New tests: `crates/smelt-logical` `partition_axis_for_type_classifies_date_integer_and_other`; `crates/smelt-runtime/tests/partition_axis_windowing.rs` (7 tests: display/sql_literal, unit-step chunking, batch-size unit counting, positive-span-only validation, domain-mismatch refusals, day-typed-widening refusal); `execute::tests::parse_run_window_accepts_integer_bounds` + a companion refusal test.

## Decisions

- Unit-step integer grid (already recorded in `outcome.md`'s 2026-09-04 entry) — no new design calls made this phase.
- Day-typed widening refusal is keyed off `EffectiveWindow`/`Skew` being nonzero, not off `BatchSafety::BoundedSafe`'s `max_chunk_days` — that field is already expressed in the partition column's own unit (per the existing spec text on batch-safety classification), so it needs no axis-specific gating.
- `frozen_horizon` clamp math (day-count arithmetic) stays calendar-axis-only this phase; an integer-axis model with `frozen_horizon` configured is unclamped rather than refused — no fixture exercises the combination, and forcing a refusal here would be scope creep into 5b/6's territory. Flagged for a later phase if it matters.
- The global `parse_run_window` (used by every calendar-only consumer — key-grain dispatch, `PartitionRange`, etc.) stays `Option<NaiveDate>`-typed and mostly unchanged; it now tolerates a non-calendar-shaped-but-valid-increasing-integer-pair by returning `(None, None)` instead of hard-erroring, so an integer-axis-only run doesn't die before axis resolution gets a chance. A new `parse_run_window_in_axis` (tested directly) is the per-model generalization `build_model_plans` actually uses.

## For the next planner

- **The `probe_integer_partition_column_run` residue did NOT move off the DELETE-literal `INT32` conversion error**, and this is a real, understood gap, not a bug in the axis machinery: the probe fixture's `batch_id` column threads through `SELECT batch_id, event_ts, id FROM smelt.seed_events` where `seed_events` is itself a raw `VALUES` literal. `resolved_model_schema` currently infers `Unknown(Dynamic)` for `batch_id` in that exact shape (confirmed by instrumenting `resolve_partition_axes` and observing `Unknown(Dynamic)` for all three columns). Because the type can't be resolved, axis resolution falls back to "the axis implied by the run-window literal's own form" (per this phase's spec'd fail-open design) — and the probe's windowed run passes `--event-time-start 2026-01-01` (calendar-shaped), so the fallback picks `Calendar`, and the run proceeds down the old, still-broken calendar path.
- **Verified the mechanism itself is correct**: adding an explicit `CAST(batch_id AS INTEGER)` to the probe model's SQL makes `resolved_model_schema` return `Integer`, axis resolution correctly picks `Integer`, and the same windowed run now fails with `expected a bare integer for a unit-step integer partition axis, got '2026-01-01'` — the intended, clean fail-loud refusal, at plan-construction time, before any DELETE is emitted.
- **5b (or whoever revisits the probe) needs one of**: (a) fix the underlying type-inference gap for a `VALUES`-literal column passed through one level of `smelt.ref()` indirection with no metadata sidecar — likely a pre-existing limitation unrelated to this phase's scope; or (b) accept that the probe's fixture needs an explicit cast/type declaration to exercise the intended axis-refusal path, and update the probe/fixture accordingly when 5b lands its emission work.
- `sql_literal()` exists on `PartitionPoint` but is unused this phase (no call site wired) — 5b's job per the plan.

## Gates

- `cargo test -p smelt-logical --lib partition_axis` — PASS (2 tests)
- `cargo test -p smelt-runtime --lib windowing` — PASS (0 direct; covered by integration tests below)
- `cargo test -p smelt-runtime` — PASS (all suites, including new `partition_axis_windowing.rs`, 7/7)
- `cargo test -p smelt-runtime --test statement_parity` — PASS (33/33, unchanged)
- `cargo test -p smelt-cli --test rebuild_dry_run` — PASS (4/4, unchanged)
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (75/75, unchanged)
- `cargo test -p smelt-cli --test partition_residue_probes probe_integer_partition_column_run` — PASS (still red as designed; failure mode analyzed above, did not move off the INT32 error for the literal fixture, though the mechanism is proven correct via a manual CAST-based repro)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (one run hit a pre-existing flaky `test_cli_ui_manifest_parity` under full-workspace parallel load, unrelated to this phase — reproduced clean on immediate rerun and in isolation 3/3 times; not caused by this phase's changes)
