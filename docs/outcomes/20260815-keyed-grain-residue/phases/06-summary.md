# Phase 6 summary — Un-rot the gated conformance twin (`gate_composed.rs`)

## Shipped

- Fixed `crates/smelt-maintenance-testkit/src/families/gate_composed.rs:343` — the call to
  `run_windowed_keyed_maintenance` was missing the `write_pin: Option<&'static WritePattern>`
  argument (added in a prior phase, sitting between `suppression` and `compile_step`). Inserted
  `None` with a one-line comment ("The route-3 composed recipe stages no
  `maintenance.cells[].write` pin.") — this was the single site causing all 5 reported errors
  (arg-count mismatch cascading into type mismatches for every later positional argument).
- Added a per-PR CI guard: "Gated conformance twin compile check" step in the `Lint` job
  (`.github/workflows/test.yml`, after "Run clippy") running
  `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets`.
  Compile-only, no live Spark/BigQuery needed.
- Added a comment above `#![cfg(any(feature = "spark", feature = "bigquery"))]` in
  `crates/smelt-maintenance-testkit/src/families/mod.rs` naming the new CI step that keeps the
  module compiling.

## Decisions

- Only `None` was inserted at the call site; no other rot was found once the first error cleared
  — all 5 reported errors resolved from this single fix (confirmed by re-running the check
  clean). No production signature was touched.
- Guard placed in the `Lint` job (`test.yml`), not the gated `spark-parity` job in `compat.yml`,
  per the plan — the whole point is that it runs on every PR, not just when `run-docker-tests` is
  applied.

## For the next planner

- No new limitations discovered. Phase 6's scope (compile-fix + CI guard) is now fully closed;
  ready to hand to phase 7 (non-DuckDB `Grade::Idempotent` ledger fail-loud skip) and phase 8
  (final `/smelt:validate` sweep).

## Gates

- `cargo check -p smelt-maintenance-testkit --features spark --all-targets` — RED before fix (5
  errors: E0061 missing/extra-arg mismatch cascading into 2× E0277 closure-vs-`RetryPolicy`
  mismatches), GREEN after.
- `cargo check -p smelt-maintenance-testkit --features bigquery --all-targets` — GREEN.
- `cargo check -p smelt-cli --tests --features smelt-cli/spark` — GREEN.
- `cargo check -p smelt-cli --tests --features smelt-cli/bigquery` — GREEN.
- `cargo test -p smelt-maintenance-testkit --features spark` — 69 passed, 0 failed (includes
  `composed_route3_delta_sql_is_byte_identical_for_duckdb_under_the_staged_query_shape`).
- `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets` (the new
  guard's exact command) — GREEN.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed, 0 failed (unchanged count,
  default-feature gate untouched).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
