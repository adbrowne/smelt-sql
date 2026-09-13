# Phase 6 summary — the `Backend` trait impl over the live tier

## Shipped

- `crates/smelt-backend-trino/src/backend.rs` — `TrinoBackend`, a `Backend` impl driving
  `TrinoClient` over `execute_sql`, `create_table_as`/`create_view_as` (DROP-then-CREATE, since
  `supports_create_or_replace_{table,view}` are unmeasured), `drop_table_if_exists`/
  `drop_view_if_exists`, `get_row_count`/`get_preview`, `table_exists` (catalog-scoped
  `information_schema.tables`), `ensure_schema`, `dialect` (`SqlDialect::Trino`), a provisional
  all-`false` `capabilities()`, and a refusing `load_table`. Identifier quoting
  (`qualified_name`/`quote_identifier`) and string-literal escaping are local pure functions,
  unit-tested with no server.
- `smelt-backends`: `BackendType::Trino` now constructs a `TrinoBackend` from `host`/
  `effective_trino_port`/`tls`/`user`/`catalog`/`schema`/interpolated `password`, refusing a
  target missing `host`/`user`/`catalog` before any network call. `smelt-backend-trino` is a
  non-optional dependency (no feature gate — see the outcome's decision log).
- `crates/smelt-backend-trino/tests/backend_live.rs` — 7 tests against the real tier, each
  skipping green when `SMELT_TRINO_URL` is unset: schema idempotency, table existence
  transitions, row count + preview, a view read back through a base table, `execute_model` for
  both materializations, a typed error on a bad statement, and `DROP … IF EXISTS` on a table
  that never existed.
- `crates/smelt-backends/tests/create_backend.rs` — `factory_constructs_a_trino_backend_from_a_target`
  (unconditional — construction makes no network call) and `factory_error_never_contains_the_password`.

## Decisions

- Refused `delete_partitions`/`insert_into_from_query`/`insert_overwrite` by name rather than
  porting DuckDB's emulation — see the outcome's decision log, 2026-09-14. These belong to
  `20260913-trino-incremental`.
- `create_table_as`/`create_view_as` both DROP-then-CREATE rather than assuming `OR REPLACE` —
  matches the plan's explicit guidance for tables and extends the same reasoning to views.
- `TrinoBackend::new` performs no I/O; the coordinator is only reached on the first real query —
  needed for the factory test to run unconditionally, and matches "no network call made" in the
  plan's test 5.

## For the next planner

- Live legs surfaced a real, previously-unmeasured fact: Trino types a `VALUES` integer literal
  as `integer` (Int32), not `bigint` — only `count(*)` is reliably bigint. Worth keeping in mind
  for phase 7's seed-type round-trip tests and phase 8's capability probes, where an assumed
  `bigint` from a literal would silently mis-decode.
- `DELETE nextUri` on early drop/abort (carried over from phase 5's handoff) is still not
  implemented in `TrinoClient` — still not free to add here either; every call in this phase's
  live tests runs to completion, so no early-abort path was exercised. Left for whichever phase
  next needs bounded/streaming reads.
- Phase 7 (`load_table`) and phase 8 (measured `capabilities()` / `trino_iceberg()`) are next in
  sequence; both have a clean seam here (the docstring in `backend.rs` names exactly which
  methods they replace).

## Gates

- `cargo test -p smelt-backend-trino -p smelt-backends` (SMELT_TRINO_URL unset) — unit/stub
  tests green, live tests skip.
- `cargo test -p smelt-backend-trino --test backend_live` against a live tier
  (`scripts/trino-up.sh` / `trino-env.sh` / `trino-down.sh`) — 7/7 passed, not skipped.
- `cargo test -p smelt-core --test hardening_budget` — 5/5 green, no ratchet lowered.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
