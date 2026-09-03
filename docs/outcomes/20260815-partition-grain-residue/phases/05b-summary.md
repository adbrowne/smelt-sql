# Phase 5b summary — integer-axis emission end-to-end

## Shipped

- Single-owner literal renderer: `partition_literal(axis, value) -> Result<String, String>` and
  `Region::for_axis(axis, start, end)` in `crates/smelt-logical/src/maintenance/emit.rs` — the
  only place partition-literal quoting is decided.
- `smelt_backend::PartitionRange` gained an `axis: PartitionAxis` field, threaded from the
  batch's own `PartitionPoint::axis()` at every real construction site
  (`execute.rs`'s batch loop, the T3 delta-restricted branch, the DuckDB-dialect statement
  report). All eight `format!("'{}'", …)` `Region`/DELETE sites in `execute.rs`,
  `smelt-backend/src/lib.rs`, `smelt-backend-bigquery`, `smelt-backend-spark`, and
  `smelt-backend-duckdb` now route through the renderer; the three direct DELETE predicate
  builders (`smelt-backend-duckdb/src/lib.rs`, `smelt-backend-bigquery/src/sql.rs`,
  `smelt-backend-spark/src/sql.rs`) do too, returning `Result` where they used to return `String`.
- `smelt_runtime::transformer::TimeRange` gained an `axis: PartitionAxis` field;
  `inject_time_filter` and `inject_source_filters`/`wrap_source_ref_with_filter` render through
  `partition_literal` instead of hardcoded quoting (the date-arithmetic helpers stay
  axis-blind — they're a no-op on the integer axis since day-typed widening is refused at plan
  construction, per 5a).
- `build_model_plans` (`execute.rs`) refuses `contract.frozen_horizon` on an integer-axis model
  (`ModelDefOverrideRequiresIncremental`-style fail-loud error naming the model and the field)
  instead of silently leaving it unclamped.
- `smelt_runtime::windowing::axis_implied_by_literal_form` — the "axis implied by the run-window
  literal's own form" fallback, hoisted out of `execute.rs` into one shared helper, reused by
  `smelt-cli`'s `commands/explain.rs::build_derived_window` so `--period 1..4` selects the
  integer axis. `parse_period` now accepts `<int>..<int>` alongside `YYYY-MM-DD..YYYY-MM-DD`.
  `explain.rs`'s `DerivedWindow` carries the resolved `axis`; its own DELETE/INSERT-region and
  output-clamp construction route through it.
- `smelt-cli`'s `run.rs` pre-flight bound check now accepts a bare integer as well as a calendar
  date (was hard-coded `YYYY-MM-DD`-only, which blocked every integer-axis run before axis
  resolution even ran).
- `probe_integer_partition_column_run` inverted: stages an integer-`batch_id` model (explicit
  `CAST(batch_id AS INTEGER)` — see "For the next planner" below), drives a first run, a
  `--batch-size 1` windowed backfill, and a steady-state re-run through the real `smelt` binary,
  and asserts the resulting table equals a full-refresh oracle.
- New tests: `partition_literal_renders_per_axis` / `region_for_axis_renders_per_axis`
  (`smelt-logical`), `inject_time_filter_renders_integer_bounds_bare` /
  `inject_source_filters_renders_integer_bounds_bare` / `calendar_time_filter_is_byte_identical`
  (`smelt-runtime::transformer`), `integer_axis_region_is_bare` (`smelt-runtime::execute`),
  `integer_axis_frozen_horizon_is_refused` (`smelt-runtime/tests/contract_frozen_horizon_clamp.rs`),
  `explain_period_implies_integer_axis` (`smelt-cli/tests/explain_show_sql.rs`).
- Spec: `docs/specs/incremental_shapes.md` rule 8a extended with the per-axis literal-rendering
  rule and the `frozen_horizon` integer-axis refusal; the "Monotone-integer `partition_column`
  has no end-to-end run" Known Divergences bullet removed.

## Decisions

- `PartitionRange`/`TimeRange` gained a required `axis` field (not `Default`-derived) — every
  construction site is explicit about its axis, so a future non-calendar caller can't silently
  inherit a wrong default via `..Default::default()`. ~50 call sites (mostly tests) updated
  mechanically; five needed a real (non-`Calendar`) axis: `execute.rs`'s batch loop (from
  `batch.partition_start.axis()`), the T3 delta-restricted-recompute branch, `build_model_plans`'s
  window-bound catch-all arm, and `commands/explain.rs`'s `build_derived_window`.
- `wrap_source_ref_with_filter`'s two literal parameters are now pre-rendered strings (quoted or
  bare), not bare values the function quotes itself — the function's own `'{}'` format was
  double-quoting an already-quoted literal until fixed (caught by `statement_parity` going red
  with `''2025-01-11''`-shaped SQL; the double-quote bug would have shipped without that
  regression guard).
- `run.rs`'s bound pre-flight check loosened to accept a bare integer, matching
  `axis_implied_by_literal_form`'s two recognized shapes, rather than special-casing "skip the
  check for non-date-shaped input."
- The `no_maintenance_statement_authoring_outside_the_emitter` gate's
  `STATEMENT_AUTHORING_ALLOWLIST` entries (keyed on exact literal substrings) were updated to the
  new unquoted DELETE format string — the allowlist is substring-exact by design, so any
  behavior-preserving text change to an allowlisted line requires updating its entry.

## For the next planner

- The probe fixture needs its `batch_id` column cast explicitly
  (`CAST(batch_id AS INTEGER)`) rather than relying on inference through the `VALUES`-literal →
  `smelt.ref()` hop — 5a's summary already flagged that `resolved_model_schema` infers
  `Unknown(Dynamic)` for a `VALUES`-literal column crossing one `smelt.ref()` hop. This phase
  didn't fix that inference gap (still pre-existing, still out of scope — no success criterion
  depends on it); it just worked around it in the fixture, per 5a's option (b).
  `examples/` sweep: no integer-axis partition model exists in any example, confirming the plan's
  expectation — no example changes needed.
- `docs/specs/incremental_shapes.md` bullet #3 in the phase-1 audit ("per-source clamp
  observability claims 'specified ahead of a tracking plan' but is actually tracked by
  `docs/plans/20260704-model-updates-l4-batched.md` Phase BL8") is still open — phase 6 or 8's
  close-out should fix that stale claim.
- Editor hover on a `smelt.<path>` reference showing the resolved clamp (success criterion 6,
  second half) is untouched by this phase — phase 6's scope.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature
  sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity` — PASS (33/33).
- `cargo test -p smelt-cli --test rebuild_dry_run` — PASS (4/4).
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (75/75).
- `cargo test -p smelt-cli --test partition_residue_probes probe_integer_partition_column_run` —
  PASS (inverted from red).
