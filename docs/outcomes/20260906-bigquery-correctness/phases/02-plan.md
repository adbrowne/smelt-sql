# Phase 2 plan — the rest of the dialect-blind fingerprint SQL

## Objective

Remove the remaining DuckDB-hardcoded SQL from the fingerprint/repair emitter family so
the statements a non-DuckDB target *would* execute are well-formed under its own dialect:
`key_expr_for_columns`' `CAST(... AS VARCHAR)` (GoogleSQL has no `VARCHAR` type at all)
and `emit_repair_group_digest_select`'s DuckDB-only `bit_xor(hash(...))` plus the same
`VARCHAR` cast. Advances criterion 1's substance (the phase-1 fix left the *same emitted
statement* invalid on BigQuery because of its `delta_key` half) and criterion 3 (each fix
lands with an offline gate). The runtime `supports_fingerprint_sidecar` refusal is the
"refuse loudly" leg; this phase pins it with a test so the capability cannot be flipped on
silently.

## Spec delta

None. This is emitted-SQL internals below the user-visible surface —
`docs/specs/sources.md` §"The fingerprint sidecar" specifies the *digest/diff semantics*,
not a dialect spelling, and greps clean for `VARCHAR`/`bit_xor`/backend names. The
per-dialect spelling table belongs in the emitter's doc comments, which this phase updates
(including retiring phase 1's "see phase 2 for that residue" pointer once the residue is
gone).

## Design notes the implementer must not re-litigate

- **Sidecar digests never cross engines.** A stored sidecar row is only ever compared
  against a digest computed by the same backend over the same source, so a per-dialect
  hash spelling needs no cross-engine value agreement — only well-formedness and
  order-insensitivity within one dialect. (Contrast the `dialect_audit` value leg.)
- **`key_expr_for_columns`' literal-value contract survives.** Single-column keys stay
  un-hashed; only the cast type name becomes dialect-dependent, via the existing
  `probe_dialect_string_type` (`VARCHAR`/`STRING`/`STRING`, BigQuery leg already
  live-confirmed by `scripts/bigquery-probe3.sh`).
- **Per-dialect XOR-combine spellings:** DuckDB `bit_xor(hash(...))` (unchanged);
  BigQuery `BIT_XOR(FARM_FINGERPRINT(...))` (GoogleSQL's `BIT_XOR` is INT64-only, and
  `FARM_FINGERPRINT` is its STRING→INT64 hash); Spark `bit_xor(xxhash64(...))`. Each is
  cast to `probe_dialect_string_type(dialect)`, not `VARCHAR`.
- **`KEY_NULL_SENTINEL` and the tag constants are unchanged** — this phase changes type
  names and hash spellings only, never the digest construction, or phase 1's
  `digest_select_matches_row_fingerprint_expr_for_every_dialect` would fail.

## Tests (red-green)

In `crates/smelt-logical/src/maintenance/emit/fingerprint.rs`'s existing `mod tests`:

1. `key_expr_uses_dialect_string_type_for_every_dialect` — single-column `delta_key`
   casts to `VARCHAR` on DuckDB and `STRING` on BigQuery/Spark; no `VARCHAR` survives in
   the non-DuckDB output.
2. `multi_column_key_expr_uses_dialect_string_type` — the composite-key branch
   (`concat_varchar_expr_typed`) is dialect-parameterised too, not just the single-column
   branch.
3. `digest_select_delta_key_is_googlesql_clean` — the *whole* statement from
   `emit_fingerprint_digest_select(.., BigQuery)` contains no `VARCHAR` anywhere (the
   statement-level assertion phase 1 could not yet make).
4. `repair_group_digest_uses_duckdb_bit_xor_hash` — DuckDB output is byte-identical to
   today's (no regression in the only live path).
5. `repair_group_digest_uses_googlesql_bit_xor_farm_fingerprint` — BigQuery output uses
   `BIT_XOR(FARM_FINGERPRINT(` and `AS STRING`, and contains no `VARCHAR`/`hash(`.
6. `repair_group_digest_uses_spark_xxhash64` — Spark output uses `xxhash64` and
   `AS STRING`.
7. `key_addressed_affected_keys_select_threads_its_dialect` — the third `_dialect`-ignoring
   emitter in this file also honours its parameter.

In `crates/smelt-runtime/tests/fingerprint_sidecar.rs`:

8. `sidecar_capability_is_declared_only_where_the_digest_sql_is_verified` — asserts
   `supports_fingerprint_sidecar` is `true` for DuckDB only, with a comment naming what a
   flip requires (a live value-leg sweep for that engine). This is the loud-refusal gate:
   flipping the flag without doing that work fails offline.

## Tasks

1. Add a `dialect: MaintenanceDialect` parameter to `key_expr_for_columns`; use
   `probe_dialect_string_type(dialect)` for the cast and switch the composite branch from
   `concat_varchar_expr` to `concat_varchar_expr_typed`.
2. Delete `concat_varchar_expr` if it has no remaining callers; otherwise leave it and note
   why.
3. Thread the dialect at every `key_expr_for_columns` call site:
   `emit/recompute.rs` (`emit_per_group_recompute`, `_dialect` → `dialect`),
   `emit/fingerprint.rs` (both digest selects, `emit_key_addressed_affected_keys_select`).
4. Add `dialect: MaintenanceDialect` to the smelt-runtime helpers that build key
   expressions but take no dialect today — `repair_affected_keys_select`,
   `repair_candidate_select`, `repair_slice_predicate` (`maintenance_driver/repair/execute.rs`)
   — and pass `maintenance_dialect(backend.dialect())` / the caller's existing `dialect`
   from `diagnostics/preview.rs`, `execute/project/mod.rs`,
   `maintenance_driver/key_addressed/execute.rs`, and
   `maintenance_driver/repair/execute.rs`'s own `execute_per_group_recompute`.
5. Give `emit_repair_group_digest_select` a real per-dialect body (drop the `_` prefix) per
   the spelling table above.
6. Update the doc comments: `key_expr_for_columns`, `emit_repair_group_digest_select`
   ("only the DuckDB shape is built today" is no longer true),
   `emit_key_addressed_affected_keys_select`, and `emit_fingerprint_digest_select`'s
   phase-2 residue pointer.
7. Update existing test call sites in `crates/smelt-runtime/tests/repair_lowering.rs`,
   `tests/statement_parity/repair_and_key_addressed.rs`, and the in-module tests.
8. Add test 8's capability assertion.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet` (executed-vs-emitted parity
  must survive the signature change)
- `cargo test -p smelt-runtime --test fingerprint_sidecar --test repair_lowering --test key_addressed_model_edge_lowering --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` (DuckDB output unchanged)
- No live BigQuery leg needed — every assertion is over emitted SQL text and the capability
  matrix.

## Commit message

`fix(maintenance): thread dialect through key_expr_for_columns and the repair-group digest`
