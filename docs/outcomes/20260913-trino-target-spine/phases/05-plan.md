# Phase 5 plan — `smelt-backend-trino`: the HTTP statement client

## Objective

Land a new `crates/smelt-backend-trino` whose only content is a pure-Rust client for Trino's
`POST /v1/statement` + `nextUri` protocol: page following, result pages decoded to Arrow
`RecordBatch`es, Trino error objects mapped to typed `BackendError` variants, and the `password`
credential redacted everywhere it could surface. Advances criterion 4 (the HTTP half — the
`Backend` trait impl is phase 6's) and the redaction half of criterion 1. Every test in this
phase runs against a **local stub coordinator**, never the Docker tier: the client's protocol
handling must be provable with `SMELT_TRINO_URL` unset.

## Spec delta

None. `multi_backend.md` §"Connection security" already states the `trino` credential rule
(password as a distinct key, HTTP Basic alongside `user`, never in a log line, an error message,
a diagnostic or a run report) and §"Loading data into a backend" already names `/v1/statement`
as the protocol. This phase implements them; it changes no user-visible surface.

## Tests

Red-green, all in `crates/smelt-backend-trino/`. The stub coordinator is an `axum` dev-dependency
router bound to an ephemeral port, returning canned JSON pages.

Unit (in `src/`, no server):
1. `protocol::decodes_a_finished_page` — a `QueryResults` JSON body with `columns` + `data` +
   `stats.state = "FINISHED"` and no `nextUri` deserializes with every field the client reads.
2. `arrow::maps_every_seed_type` — Trino type signatures `boolean`, `integer`, `bigint`, `real`,
   `double`, `decimal(18,4)`, `varchar`, `varchar(32)`, `date`, `timestamp(6)` each map to the
   expected Arrow `DataType`.
3. `arrow::unknown_type_is_an_error_not_a_guess` — an unrecognised Trino type signature returns
   an error naming it, never a silent `Utf8`/null column (fail-loud discipline).
4. `arrow::json_values_become_typed_arrow_values` — a page's JSON rows (including `null`s) decode
   into a `RecordBatch` with the right values and null mask, row count matching `data.len()`.
5. `error::trino_error_codes_map_to_typed_variants` — `TABLE_NOT_FOUND` → `NotFound`,
   `SCHEMA_NOT_FOUND` → `SchemaNotFound`, `NOT_SUPPORTED`/`FUNCTION_NOT_FOUND` →
   `UnsupportedFeature`, any other `USER_ERROR`/`INTERNAL_ERROR` → `ExecutionFailed` carrying
   Trino's own message. No arm produces `Other`/a stringly catch-all.
6. `redaction::debug_never_prints_the_password` — `format!("{:?}", TrinoClientConfig { password:
   Some("hunter2"), .. })` contains `REDACTED` and not `hunter2`.

Stub-server (`tests/statement_client.rs`):
7. `follows_next_uri_to_the_end` — three pages (data + `nextUri`, data + `nextUri`, data + no
   `nextUri`) yield all rows concatenated, in order.
8. `queued_pages_with_no_columns_are_not_an_empty_result` — a `QUEUED` page carrying `nextUri`
   but neither `columns` nor `data` is followed rather than returned as a zero-column result.
9. `an_error_page_is_never_a_silent_empty_result` — a page whose `error` object is set returns
   `Err`, even when it also carries `columns` and an empty `data`.
10. `sends_the_trino_session_headers` — the stub asserts `X-Trino-User`, `X-Trino-Catalog`,
    `X-Trino-Schema` and, when a password is configured, an `Authorization: Basic` header.
11. `a_refused_connection_is_connection_failed` — pointing the client at a closed port yields
    `BackendError::ConnectionFailed`, and its `to_string()` contains no password.
12. `an_http_5xx_is_not_parsed_as_a_result_page` — a 503 with an HTML body maps to a typed error
    naming the status, not a JSON parse panic.

## Tasks

1. Create `crates/smelt-backend-trino` (`Cargo.toml`: `smelt-backend`, `arrow`, `reqwest`
   (`default-features = false`, `rustls-tls`, `json`), `serde`, `serde_json`, `tokio`,
   `async-trait`, `thiserror`, `anyhow`, `tracing`; dev: `axum`, `tokio` test features). No
   `Backend` impl yet — phase 6 adds it.
2. `src/protocol.rs` — serde types for `QueryResults`, `Column`, `ClientTypeSignature`,
   `StatementStats`, `QueryError`/`ErrorLocation`, with `#[serde(rename_all = "camelCase")]` and
   tolerant unknown-field handling. Tests 1, 5.
3. `src/config.rs` — `TrinoClientConfig { base_url, user, catalog, schema, password }` with a
   hand-written `Debug` that redacts `password`. Test 6.
4. `src/arrow_convert.rs` — Trino type signature → Arrow `DataType`, and a page's JSON `data`
   rows → `RecordBatch`. Unrecognised type returns `Err`. Tests 2, 3, 4.
5. `src/error.rs` — `QueryError` → `BackendError`; transport/status failures → `ConnectionFailed`
   / `ExecutionFailed` with the status named. Test 5, 12.
6. `src/client.rs` — `TrinoClient::execute(sql) -> Result<Vec<RecordBatch>, BackendError>`:
   `POST /v1/statement` with the session headers, then follow `nextUri` with `GET` until absent,
   checking `error` on every page and accumulating pages that carry `columns` + `data`. Honour
   `DELETE nextUri` on drop/abort only if free; otherwise leave for phase 6. Tests 7–11.
7. `tests/statement_client.rs` — the `axum` stub coordinator harness plus tests 7–12.
8. Run the hardening gate; if the new crate is counted, add its entry via
   `.claude/scripts/hardening-budget.sh --update` (aim for zero production `unwrap`/`expect` and
   zero `println!` so the entry is trivial).
9. Write `phases/05-summary.md`.

## Verification

- `cargo test -p smelt-backend-trino` — all tests green with `SMELT_TRINO_URL` unset.
- `bash .claude/scripts/verify-phase.sh` — fmt, clippy (both feature sets), shellcheck, workspace
  tests, example_diagnostics.
- `cargo test -p smelt-core --test hardening_budget` — the new crate must not lower a ratchet.

## Commit message

`feat(trino): land the smelt-backend-trino HTTP statement client with typed error mapping`
