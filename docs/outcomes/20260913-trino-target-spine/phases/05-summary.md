# Phase 5 summary — `smelt-backend-trino`: the HTTP statement client

## Shipped

- New crate `crates/smelt-backend-trino/` with no `Backend` trait impl yet (phase 6):
  - `src/protocol.rs` — tolerant serde types for `QueryResults`/`Column`/`ClientTypeSignature`/
    `StatementStats`/`QueryError`/`ErrorLocation`.
  - `src/config.rs` — `TrinoClientConfig` with a hand-written `Debug` that redacts `password`.
  - `src/arrow_convert.rs` — `trino_type_to_arrow` (10 seed types, unrecognised signature →
    typed error) and `rows_to_record_batch` (JSON rows + nulls → Arrow `RecordBatch`, one
    builder per Arrow type via a small macro for the scalar cases, explicit loops for
    date/timestamp/decimal string parsing).
  - `src/error.rs` — `map_trino_error` (exhaustive on the five named codes, `_` arm still
    carries Trino's own message via `ExecutionFailed`) and `map_transport_error`.
  - `src/client.rs` — `TrinoClient::execute`: `POST /v1/statement`, follows `nextUri` with
    session headers + optional Basic auth on every request, checks `error` before `columns`/
    `data` on every page, treats a `nextUri`-only page (no columns/data) as "keep paging" not
    "empty result".
  - `tests/statement_client.rs` — an `axum` stub coordinator (dev-dependency) proving all 6
    stub-server tests from the plan.
- `.claude/hardening-baseline.txt` — added `smelt-backend-trino {expect,println,unwrap} 0`
  entries by hand (not via `--update`, see Decisions).

## Decisions

- **Baseline updated by hand, not `--update`.** `hardening-budget.sh --update` regenerates the
  whole file and silently drops the accumulated sign-off comment history at the top (5 dated
  entries documenting prior ratchet raises). Ran `--update` once, saw the diff would delete that
  history for a change that only *adds* three zero-count lines, reverted with `git checkout --`,
  and inserted the three `smelt-backend-trino` lines by hand instead. No sign-off note needed —
  no existing count changed.
- **Trino error → `BackendError` mapping keeps `schema`/`model` fields empty and puts Trino's
  message in the field the caller actually reads** (`NotFound { schema: "", name: message }`,
  `execution_failed("trino", message)`) rather than trying to parse `schema.table` back out of
  Trino's English error text — the message is preserved verbatim either way, and guessing a
  structured field from prose text seemed more fragile than useful in this phase.
- **Timestamp maps to `Timestamp(Microsecond, None)`** (no timezone) since the seed type set's
  `timestamp(6)` is the no-timezone Trino type; `timestamp with time zone` is out of this
  phase's scope.
- **Session headers sent on every request, not just the initial `POST`.** Real Trino accepts
  this even though only the first request strictly needs it; simpler than special-casing.

## For the next planner

- Phase 6 (`Backend` trait impl) needs `DELETE nextUri` on drop/abort, deliberately deferred
  here per the plan's task 6 ("Honour `DELETE nextUri` on drop/abort only if free; otherwise
  leave for phase 6") — `TrinoClient` currently never issues it, so a query result set is never
  explicitly released early on the coordinator side. Worth confirming this doesn't leak
  coordinator-side query state across a long-running session.
- `UnknownTrinoType`/decimal-parse/date-parse errors are all folded into `BackendError::
  ExecutionFailed { model: "trino", .. }` rather than a distinct variant — fine for now since
  `is_transient()` already treats `ExecutionFailed` as deterministic (correct), but if phase 8's
  capability-probe work wants to distinguish "protocol/decode bug in our client" from "the SQL
  itself was rejected" it may want a dedicated variant.
- Not implemented: connection pooling / retry — `TrinoClient::new` builds a fresh
  `reqwest::Client` per instance with defaults; phase 6 should decide whether the `Backend` impl
  shares one client across calls (current code already supports this — `execute` takes `&self`).

## Gates

- `cargo test -p smelt-backend-trino` — 11 tests green (5 unit + 6 stub-server), `SMELT_TRINO_URL`
  never read.
- `cargo test -p smelt-core --test hardening_budget` — 5/5 green, no ratchet lowered or raised.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
