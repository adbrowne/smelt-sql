# Phase 5b — Integer-axis emission end-to-end

**Row:** 5b · **Success criteria advanced:** 5 (completes it), and the criterion-8 divergence removal for this bullet

## Objective

Phase 5a made the *axis* typed but left every emitted partition literal hard-quoted
(`format!("'{}'", …)` at eight `Region`-construction sites plus three backend predicate
builders) and left `smelt explain` pinned to `PartitionAxis::Calendar`. This phase gives the
integer axis a single-owner literal renderer, threads it through scan-filter injection, the
DELETE/INSERT region and the explain clamp, and inverts
`probe_integer_partition_column_run` into a real first-run / backfill / steady-state proof
against a full-refresh oracle.

## Spec delta (spec-first — the implement step makes these edits before code)

1. `docs/specs/incremental_shapes.md` §"The partition grain" Rules — extend rule 8a: a partition
   literal is rendered **in the axis's own domain** — quoted (`'2026-01-01'`) on the calendar
   axis, bare (`7`) on the integer axis — everywhere a run emits one (the output clamp, the
   per-source scan filter, the maintenance region's DELETE predicate) and everywhere `smelt
   explain` reports one. `--period` bounds are read in the same domain.
2. Same file, §"The partition grain" Rules — a `contract.frozen_horizon` on an integer-axis model
   is a hard refusal (its horizon is a day count with no conversion into partition units),
   consistent with rule 8a's fail-closed treatment of day-typed widening inputs.
3. Same file, §Known Divergences — remove the **"Monotone-integer `partition_column` has no
   end-to-end run"** bullet.

## Tests (red-green)

- `smelt-logical` (`maintenance::emit`): `partition_literal_renders_per_axis` — `Calendar` quotes
  and escapes; `Integer` renders bare; a non-integer value on the integer axis is `Err`.
- `smelt-runtime::transformer`: `inject_time_filter_renders_integer_bounds_bare` — an integer-axis
  `TimeRange` produces `batch_id >= 1 AND batch_id < 2`, no quotes.
- `inject_source_filters_renders_integer_bounds_bare` — same for the wrapped source-ref filter.
- `calendar_time_filter_is_byte_identical` — the existing calendar output is unchanged.
- `smelt-runtime::execute`: `integer_axis_region_is_bare` — the `Region` built for an
  integer-axis batch carries bare literals.
- `integer_axis_frozen_horizon_is_refused` — an integer-axis model with `contract.frozen_horizon`
  errors at plan construction naming the model and the field.
- `smelt-cli` `explain`: `explain_period_implies_integer_axis` — a bare-integer `--period` renders
  the derived window and clamp as bare integers.
- `smelt-cli` `partition_residue_probes::probe_integer_partition_column_run` — **inverted**:
  first run (no window), a windowed backfill with `--batch-size`, and a steady-state re-run all
  succeed, and the final table equals a full-refresh oracle.
- Regression guards (must stay green untouched): `-p smelt-runtime --test statement_parity`,
  `-p smelt-cli --test rebuild_dry_run`, `-p smelt-cli --test maintenance_conformance`.

## Tasks

1. Make the three spec edits above.
2. Add the single-owner renderer in `smelt-logical`'s maintenance layer:
   `partition_literal(axis, value) -> Result<String>` plus `Region::for_axis(axis, start, end)`.
   Per the maintenance-plan purity rule the renderer lives here, never in a backend.
3. Replace all eight `Region { start: format!("'{}'", …), … }` sites with `Region::for_axis`:
   `execute.rs` ×4 (lines ~950, ~4066, ~4153, ~4247), `smelt-backend/src/lib.rs`,
   `smelt-backend-bigquery/src/lib.rs`, `smelt-backend-spark/src/lib.rs`.
4. Carry the axis to those sites: add `axis: PartitionAxis` to `smelt_backend::PartitionRange`
   (constructed from the batch's `PartitionPoint`s in `execute.rs`) and use it in the three
   direct predicate builders that quote today (`smelt-backend-duckdb/src/lib.rs:583`,
   `smelt-backend-bigquery/src/sql.rs:104`, `smelt-backend-spark/src/sql.rs:75`).
5. Add the axis to `smelt_runtime::TimeRange` (or a rendered-literal pair alongside `start`/`end`)
   and route `inject_time_filter` / `inject_source_filters` / `wrap_source_ref_with_filter`
   through the renderer instead of their own `'{}'` formatting. `subtract_seconds_from_date` /
   `add_seconds_to_date` stay calendar-only — an integer axis with nonzero widening is already
   refused in 5a, so assert that rather than converting.
6. Refuse `contract.frozen_horizon` on an integer axis in `build_model_plans` (today it is
   silently skipped by the `(Date, Date)` match arm — a fail-loud gap 5a flagged).
7. Hoist 5a's inline "axis implied by the run-window literal's form" fallback into one shared
   helper and reuse it in `build_derived_window` (`smelt-cli/src/commands/explain.rs:894`) so a
   bare-integer `--period` selects `PartitionAxis::Integer`; drop the hardcoded `Calendar` and its
   stale comment. Confirm `--period` parsing admits bare-integer bounds.
8. Rewrite the probe fixture: keep the `VALUES` seed but project
   `CAST(batch_id AS INTEGER) AS batch_id` so `resolved_model_schema` resolves the axis for real
   rather than via the literal fallback (the underlying `Unknown(Dynamic)` inference gap for a
   `VALUES` column crossing one `smelt.ref()` hop is pre-existing and out of scope — record it in
   the summary). Invert the probe to the three-run + oracle assertion above.
9. Sweep `examples/` for any integer-axis partition model; none is expected — note the result.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test rebuild_dry_run`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --test partition_residue_probes probe_integer_partition_column_run`
  — now GREEN (inverted).

## Commit message

`feat(runtime): axis-domain partition literals and an end-to-end integer-axis run`
