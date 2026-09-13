# Phase 7 plan — `load_table`: the Arrow load path over the seed type set

## Objective

Replace `TrinoBackend::load_table`'s phase-6 refusal with a working Arrow load path over
Trino's HTTP protocol, covering the whole seed type set of `seeds.md` §"Type inference",
rejecting NULLs in a non-nullable Arrow field, and round-tripping every supported type back
to the same smelt `DataType` through the live tier. Advances **criterion 6** (data gets in,
bulk strategy measured and recorded, `seed_parity` covers Trino) and closes the last
`Backend` method this outcome owns.

## Spec delta

`docs/specs/multi_backend.md` §"Loading data into a backend" — the Trino paragraph currently
says the bulk path "is measured rather than assumed, and named here once phase 7 …
measures it". Replace that forward reference with the measured answer: which path smelt
takes over `/v1/statement`, the statement shape, the row-chunking rule, and the measured
number behind the choice. The §"Loading data into a backend" invariant (no host-filesystem
assumption) already covers Trino and needs no change.

## Tests

Unit (no server, in `smelt-backend-trino`):

1. `arrow_type_to_trino_type_covers_the_seed_type_set` — every Arrow type the `Backend::load_table`
   doc-comment lists maps to a Trino DDL type; `Timestamp(us, None)` maps to `timestamp(6)`
   (not bare `timestamp`, so the read-back precision is exact), `Utf8` to unbounded `varchar`.
2. `arrow_type_to_trino_type_refuses_an_unsupported_type` — an Arrow type outside the seed set
   (e.g. `Float32`, `List`) is a typed `BackendError`, never a silent `varchar` fallback.
3. `renders_typed_literals_for_every_seed_type` — the literal renderer emits a form Trino types
   unambiguously: `DATE '…'`, `TIMESTAMP '…'` with 6 fractional digits, a decimal string,
   quoted strings with `''` escaping, and bare `NULL` (typed by the enclosing `CAST`).
4. `rejects_null_in_a_non_nullable_column_before_any_statement` — NULL in a
   `Field::nullable == false` column returns `BackendError::null_in_non_nullable_column`
   naming column and row index, with no SQL built.
5. `chunks_rows_into_bounded_insert_statements` — a batch larger than the chunk size produces
   more than one `INSERT`, and each statement's row count is at the bound.
6. `escapes_a_string_literal_containing_a_quote` — a value with `'` round-trips through the
   renderer without breaking the statement.

Live (`tests/backend_live.rs`, skipping green when `SMELT_TRINO_URL` is unset):

7. `load_table_round_trips_the_whole_seed_type_set` — load one batch covering Boolean, Int32,
   Int64, Decimal128(18,4), Float64, Date32, Timestamp(us,None), Utf8 (each with one NULL row
   in a nullable column), read back via `execute_sql`, and assert the returned Arrow schema is
   field-for-field equal to the input schema **and** the values match.
8. `load_table_replaces_an_existing_table` — loading twice leaves only the second load's rows
   (the trait's drop-then-create contract).
9. `load_table_rejects_null_in_non_nullable_against_the_live_tier` — the rejection happens and
   no table is left behind.
10. `load_table_loads_a_multi_chunk_batch` — a row count above the chunk bound lands every row
    (`count(*)` equals the input row count); this test is also where the bulk-strategy timing
    is measured.

CLI parity (`crates/smelt-cli/tests/seed_parity.rs`):

11. `seed_loads_into_trino` — a **separate** test gated on `SMELT_TRINO_URL`, running
    `smelt seed --target trino` against a staged workspace with a `trino:` target block, then
    reading the rows back. Deliberately NOT wired into `targets_to_run()`: that helper is
    consumed by every W1+ suite, most of which run `smelt build`, and `dialect_and_capabilities`
    still refuses Trino until phase 8 — adding Trino there would turn green suites red for a
    reason unrelated to seeding.

## Tasks

1. Add `arrow_type_to_trino_type` (write direction) beside the existing `trino_type_to_arrow`
   in `arrow_convert.rs`, covering exactly the seed type set and erroring on anything else.
2. Add a per-value literal renderer for those types, with `''` escaping and bare `NULL`.
3. Implement `load_table`: validate nullability against `arrow_schema` first (same shape as
   Spark's and BigQuery's), then `DROP TABLE IF EXISTS` / `DROP VIEW IF EXISTS`, `CREATE TABLE`
   from the mapped DDL types, then chunked inserts.
4. Shape each insert as `INSERT INTO t (cols) SELECT CAST(c1 AS t1), … FROM (VALUES …) AS v(c1, …)`
   rather than a bare `VALUES` list — it gives NULLs and narrow integer literals an explicit
   type, which is exactly where phase 6's `integer`-not-`bigint` finding bites.
5. Chunk rows so a statement stays bounded; pick the bound from measurement, not from taste.
6. Measure the load: time a seed-scale load (≥10k rows) at two or three chunk sizes against the
   live tier and record the numbers.
7. Delete the `load_table_refuses_until_phase_seven` unit test and the `load_table` sentence in
   `backend.rs`'s module docstring.
8. Write the measured answer into `multi_backend.md` §"Loading data into a backend" in this
   same commit, and append the measurement to the outcome's decision log.
9. Add the `seed_parity` Trino leg (test 11), including a `trino_target_block()` helper and a
   `block_on`-based read-back path in `crates/smelt-cli/tests/common/` — the Trino client is
   async and `execute_sql_on` is not.

## Verification

- `bash .claude/scripts/verify-phase.sh` — full gate, `SMELT_TRINO_URL` unset (live legs skip).
- `bash scripts/trino-up.sh && source scripts/trino-env.sh` then
  `cargo test -p smelt-backend-trino --test backend_live` — every live test **runs**, not skips;
  paste the pass count into the summary.
- `cargo test -p smelt-cli --test seed_parity` with `SMELT_TRINO_URL` set — the Trino leg runs.
- `cargo test -p smelt-core --test hardening_budget` — no ratchet lowered.
- If the tier cannot be reached, emit `<<PHASE_BLOCKED>>`; **never** report the live legs green
  on a skip.

## Commit message

`feat(trino): implement load_table over the Arrow seed type set with a measured bulk path`
