# Phase 6 plan — the `Backend` trait impl over the live tier

## Objective

Turn `smelt-backend-trino` from an HTTP client into a `Backend`: DDL, existence, row count,
preview, `ensure_schema`, and a table *and* a view materialized as Iceberg objects on the live
coordinator and read back through `execute_model`. Wire `smelt-backends`' factory to construct it
from a `type: trino` target. Advances criterion 4 (whole trait over pure-Rust HTTP, typed errors)
and is the prerequisite for criteria 5–7.

Two required trait methods are deliberately answered provisionally, per the Decision log:
`capabilities()` returns an all-`false` profile pinned by a test (phase 8 replaces it), and
`load_table` returns a fail-loud `UnsupportedFeature` naming phase 7 (phase 7 replaces it).
`create_materialized_view_as` inherits the erroring default — no override.

## Spec delta

None. Phase 1 already specified the `trino` target, its dialect and its capability column; this
phase changes no user-visible surface beyond the factory no longer refusing a `trino` target,
which the spec already describes as supported.

## Tests

Unit / stub (no live server, run in the default `cargo test`):

1. `capabilities_are_provisionally_all_false` — every `BackendCapabilities` flag on
   `TrinoBackend` is `false`; the guard that stops the placeholder becoming an unmeasured claim.
2. `dialect_is_trino` — `Backend::dialect()` returns `SqlDialect::Trino`.
3. `load_table_refuses_until_phase_seven` — returns `BackendError::UnsupportedFeature` mentioning
   `load_table`, not a silent success.
4. `create_materialized_view_as_inherits_the_erroring_default` — `UnsupportedFeature`.
5. `factory_constructs_a_trino_backend_from_a_target` (in `smelt-backends`) — a `type: trino`
   `Target` yields a backend whose `dialect()` is `SqlDialect::Trino`, no network call made.
6. `factory_error_never_contains_the_password` — a target with a password whose construction
   fails produces a message with no credential substring (criterion 1's security rule).
7. `qualified_name_uses_catalog_schema_table` — the identifier builder emits
   `"cat"."sch"."tbl"` with double-quote quoting, and escapes an embedded quote.

Live (`tests/backend_live.rs`, each skips when `SMELT_TRINO_URL` is unset):

8. `ensure_schema_is_idempotent` — twice in a row, second is a no-op, not an error.
9. `table_exists_is_false_then_true_then_false` — around `create_table_as` / `drop_table_if_exists`.
10. `create_table_as_then_row_count_and_preview` — `CREATE TABLE AS SELECT` of known rows;
    `get_row_count` matches and `get_preview(limit)` returns exactly `limit` Arrow rows with the
    expected column names and types.
11. `create_view_as_then_read_back_and_drop` — a view over the table, queried through
    `execute_sql`, then `drop_view_if_exists`.
12. `execute_model_materializes_a_table_and_a_view` — the trait's `execute_model` default for both
    materializations, each read back; the phase's headline assertion.
13. `a_bad_statement_maps_to_a_typed_error` — `SELECT * FROM does_not_exist` yields a typed
    `BackendError` (not a stringly catch-all, not an empty `Ok`).
14. `drop_table_if_exists_on_a_missing_table_is_ok` — the `IF EXISTS` path.

## Tasks

1. Add `smelt-backend-trino` as a **non-optional** dependency of `smelt-backends`.
2. `src/backend.rs`: `TrinoBackend { client: TrinoClient, catalog, schema }` with
   `TrinoBackend::new(TrinoClientConfig)`; one `TrinoClient` shared across calls (`&self`).
3. Identifier quoting helper (double quotes, `""` escape) producing `catalog.schema.name`;
   test 7.
4. Implement the required methods over `client.execute`: `execute_sql`, `create_table_as`
   (`DROP TABLE IF EXISTS` then `CREATE TABLE … AS`, since `supports_create_or_replace_table` is
   not yet measured), `create_view_as`, `drop_table_if_exists`, `drop_view_if_exists`,
   `get_row_count` (`SELECT count(*)`, decode the single Arrow cell), `get_preview`
   (`SELECT * … LIMIT n`), `table_exists` (`information_schema.tables`, catalog-scoped),
   `ensure_schema` (`CREATE SCHEMA IF NOT EXISTS`), `dialect`, provisional `capabilities`,
   refusing `load_table`.
5. Wire `smelt-backends`: replace the `BackendType::Trino => Err(...)` arm with construction from
   the target's `host`/`effective_trino_port`/TLS/`user`/`catalog`/`schema`/interpolated password;
   confirm no error path formats the password.
6. `tests/backend_live.rs` with the `SMELT_TRINO_URL` skip guard in the `spark` shape
   (`eprintln!` + `return`), a per-run unique schema name so concurrent runs cannot collide, and
   best-effort cleanup at the end.
7. Issue `DELETE nextUri` on early drop/abort if it falls out cheaply; otherwise record it as a
   known non-leak-critical gap in the summary (carried over from phase 5's handoff).
8. Update `.claude/large-file-baseline.txt` only if a file actually regresses.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-backend-trino` and `cargo test -p smelt-backends` with `SMELT_TRINO_URL`
  **unset** — the unit/stub tests pass and the live ones skip.
- **The live legs must actually run**, not skip: `bash scripts/trino-up.sh`,
  `source scripts/trino-env.sh`, then `cargo test -p smelt-backend-trino --test backend_live`,
  and `bash scripts/trino-down.sh` after. Per this outcome's driver note, if the tier cannot be
  reached, emit `<<PHASE_BLOCKED>>` — **never** report the phase done on a green skip.
- `cargo test -p smelt-core --test hardening_budget` — no ratchet lowered; new `unwrap`/`expect`
  in the crate is a `Result` conversion, not a baseline raise.

## Commit message

`feat(trino): implement the Backend trait over the live Iceberg tier and wire the backend factory`
