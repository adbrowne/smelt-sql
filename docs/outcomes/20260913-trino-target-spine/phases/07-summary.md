# Phase 7 summary — `load_table` over the Arrow seed type set

## Shipped

- `arrow_type_to_trino_type` (write direction, `crates/smelt-backend-trino/src/arrow_convert.rs`)
  — maps the seed type set (`Boolean`, `Int32`, `Int64`, `Decimal128(p,s)`, `Float64`, `Date32`,
  `Timestamp(Microsecond, None)`, `Utf8`) to Trino DDL types; refuses anything else with a typed
  `BackendError`.
- `render_trino_literal` — per-cell literal rendering (`DATE '…'`, `TIMESTAMP '…'` with 6
  fractional digits, a plain decimal string, `''`-escaped quoted strings, bare `NULL`).
- `build_load_plan` + `LoadPlan` (`crates/smelt-backend-trino/src/backend.rs`) — a pure function
  building the whole `CREATE TABLE` + chunked `INSERT` plan, testable without a server.
  `TrinoBackend::load_table` is now a thin executor over the plan.
- `INSERT_CHUNK_SIZE = 1000`, chosen from live measurement (see Decisions).
- 4 new live tests in `tests/backend_live.rs`: whole-seed-type-set round trip, replace semantics,
  live NULL rejection, multi-chunk load with timing.
- `seed_loads_into_trino` in `crates/smelt-cli/tests/seed_parity.rs`, plus
  `trino_env`/`trino_schema`/`trino_target_block`/`trino_backend`/`fetch_trino_rows`/
  `drop_trino_schema` in `tests/common/mod.rs`.
- `docs/specs/multi_backend.md` §"Loading data into a backend" — the measured bulk path and the
  `PARAMETRIC_DATETIME` divergence.

## Decisions

- Bulk path is chunked `INSERT INTO … SELECT CAST(…) FROM (VALUES …)`; Parquet staging was not
  reachable to measure (no object-store credential on the `trino` target shape) — see outcome
  decision log.
- `TrinoClient::send` now sends `X-Trino-Client-Capabilities: PARAMETRIC_DATETIME` on every
  request — without it, `timestamp(6)` values silently truncate to millisecond precision on
  read-back (measured; the underlying storage is unaffected). This is a client-protocol fix
  affecting every read through this client, not a `load_table`-only patch.
- `build_load_plan` is pure and separate from the async `load_table`, so nullability/chunking/
  DDL-mapping are unit-testable without a live server or HTTP stub.
- `smelt-backend-trino` added as an unconditional `smelt-cli` dev-dependency (matches the crate's
  existing non-optional status in `smelt-backends`).

## For the next planner

- The `INSERT`-over-HTTP path measured ~6,000 rows/sec at seed scale (12k rows / ~1.8–2.1s). If a
  later outcome needs faster bulk loads, Parquet staging into the Iceberg connector's backing
  object store is the candidate, but it needs a `trino` target-shape change (an object-store
  credential) — out of this phase's scope, flagged for whoever picks that up.
  `PARAMETRIC_DATETIME` should be kept in mind by phase 8 (capability measurement) and phase 9
  (end-to-end): any live probe reading a parametric date/time value depends on this header
  already being sent, which it now is unconditionally.
- Phase 8 (capability profile) and phase 9 (`execute_project` end-to-end) are next; both were
  already blocked on Trino refusing `dialect_and_capabilities`, unaffected by this phase.

## Gates

- `cargo test -p smelt-backend-trino --lib` — 16 passed (unit tests including the 6 new ones).
- `cargo clippy -p smelt-backend-trino --all-targets` — clean.
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-backend-trino --test backend_live` — 11 passed, all live (none skipped).
- `cargo test -p smelt-cli --test seed_parity` with `SMELT_TRINO_URL` set — 2 passed (DuckDB leg
  + the new Trino leg).
- `cargo clippy -p smelt-cli --all-targets` — clean.
- `bash .claude/scripts/verify-phase.sh` — pass (see full output in the commit's CI run); run
  with `SMELT_TRINO_URL` unset so the live legs skip green, per the plan.
