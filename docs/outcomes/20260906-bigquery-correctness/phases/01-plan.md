# Phase 1 — The unconditional fingerprint-dialect fix

**Outcome:** `docs/outcomes/20260906-bigquery-correctness`
**Advances:** success criteria 1 (the unconditional fix), 3 (every fixed construct is gated), 7 (gates green).

## Objective

`emit_fingerprint_digest_select` (`crates/smelt-logical/src/maintenance/emit/fingerprint.rs:201`)
takes a `MaintenanceDialect` and throws it away — it names the parameter `_dialect` and
passes the literal `MaintenanceDialect::DuckDb` to `row_fingerprint_expr`, so a BigQuery or
Spark caller receives DuckDB's `sha256(CAST(... AS VARCHAR))` spelling. Thread the real
dialect through, assert the emitted expression per dialect, and record in the outcome's
decision log — with citations — whether the path is reachable on a live `mutable_snapshot`
run today.

## Spec delta

None. `row_fingerprint_expr` already builds the per-dialect shapes the spec expects
(`docs/specs/sources.md` §"The fingerprint sidecar" — "Digest"), including
`TO_HEX(SHA256(...))` for GoogleSQL, because `SHA256` returns `BYTES` there and the value
feeds a `STRING_AGG`. This phase makes the caller honour that; no user-visible behaviour
changes on DuckDB. If the implementer finds the doc comment on
`emit_fingerprint_digest_select` ("only the DuckDB shape is built today") is now false,
that comment is corrected as part of the change — it is the function's own contract text,
not spec surface.

## Tests

Red-green, all in `crates/smelt-logical/src/maintenance/emit/fingerprint.rs`'s existing
`#[cfg(test)] mod tests`:

1. `digest_select_uses_duckdb_hash_spelling_on_duckdb` — `MaintenanceDialect::DuckDb`
   still emits `sha256(` and `CAST(... AS VARCHAR)`. Pins the no-regression leg (should be
   green before and after).
2. `digest_select_uses_googlesql_hash_spelling_on_bigquery` — `BigQuery` emits
   `TO_HEX(SHA256(` and casts via `STRING`, and contains no `sha256(` lowercase DuckDB
   spelling and no ` AS VARCHAR)` inside the digest expression. **Red today.**
3. `digest_select_uses_spark_string_cast_on_spark` — `Spark` emits `sha256(` with
   `CAST(... AS STRING)`. **Red today.**
4. `digest_select_matches_row_fingerprint_expr_for_every_dialect` — for each of the three
   variants, the `delta_digest` expression in the emitted SELECT is byte-identical to
   `row_fingerprint_expr(digest_columns, dialect)`. This is the invariant that makes the
   defect unrepeatable rather than just fixed: the emitter never re-authors the hash.
5. `sidecar_diff_inherits_the_callers_dialect` — `emit_fingerprint_sidecar_diff` with
   `BigQuery` contains `TO_HEX(SHA256(`, proving the fix reaches the composed emitter and
   not only the leaf.

## Tasks

1. Add tests 1-5 above; confirm 2, 3 and 4 fail and 1 passes (`cargo test -p smelt-logical
   fingerprint 2>&1 | tail -40`).
2. Rename `_dialect` to `dialect` in `emit_fingerprint_digest_select` and pass it to
   `row_fingerprint_expr` instead of `MaintenanceDialect::DuckDb`. One-line change.
3. Correct the function's doc comment: the paragraph beginning "`dialect` is accepted for
   signature symmetry" is now wrong. Replace it with what is actually true after the fix —
   the digest expression is per-dialect via `row_fingerprint_expr`, while the *rest* of the
   emitted statement (the `delta_key` expression from `key_expr_for_columns`) is still
   DuckDB-shaped, and the runtime gate below is what keeps that from reaching a warehouse.
   Reference outcome phase 2 for that residue; do **not** fix it here.
4. Answer the reachability question and append a dated entry to the outcome's Decision log.
   The evidence to verify (do not take it on trust from this plan — check each):
   `crates/smelt-dialect/src/dialect.rs` declares `supports_fingerprint_sidecar: true` for
   DuckDB only and `false` for Spark, Spark-Connect and BigQuery; every runtime entry point
   in `crates/smelt-runtime/src/maintenance_driver/sidecar.rs` (lines ~133, ~242, ~389,
   ~493) refuses with `BackendError::unsupported` before reaching the emitter. State the
   verdict plainly — reachable or not — name the gate, and say what would have to change
   for it to become reachable.
5. Run the verification gates below.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet 2>&1 | tail -40`
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20` — the
  emitted-vs-executed parity gate; `fingerprint_backbuild.rs:127` calls this emitter
  directly and must still agree.
- `cargo test -p smelt-runtime --test fingerprint_sidecar --quiet 2>&1 | tail -20`
- No BigQuery live leg is needed: the assertions are over emitted SQL text, not execution.
  Do **not** claim the GoogleSQL spelling is warehouse-verified — it is not, and phase 2's
  or the close phase's ledger is where that claim would have to be earned.

## Summary

Write `phases/01-summary.md`: the diff, the reachability verdict with its citations, and
any further dialect-blind emitters spotted while in this file (feed phase 2 and the
phase-3 harvest).

## Commit message

`fix(maintenance): thread dialect through emit_fingerprint_digest_select`
