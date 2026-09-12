# Phase 1 summary — the unconditional fingerprint-dialect fix

**Shipped:**
- `crates/smelt-logical/src/maintenance/emit/fingerprint.rs`:
  `emit_fingerprint_digest_select` now threads its real `dialect` parameter into
  `row_fingerprint_expr` instead of hardcoding `MaintenanceDialect::DuckDb`. BigQuery
  callers now get `TO_HEX(SHA256(...))`; Spark gets `sha256(... CAST AS STRING ...)`;
  DuckDB behavior is unchanged (still `sha256(... CAST AS VARCHAR ...)`).
- Doc comment on the function corrected: no longer claims "only the DuckDB shape is
  built today" for the digest half. It now names the still-DuckDB-shaped residue
  (`key_expr_for_columns`'s `delta_key`/`CAST(... AS VARCHAR)`) explicitly and points to
  phase 2, plus cites the reachability verdict below.
- 5 new unit tests in the existing `#[cfg(test)] mod tests`:
  `digest_select_uses_duckdb_hash_spelling_on_duckdb`,
  `digest_select_uses_googlesql_hash_spelling_on_bigquery`,
  `digest_select_uses_spark_string_cast_on_spark`,
  `digest_select_matches_row_fingerprint_expr_for_every_dialect` (asserts the emitted
  `delta_digest` is byte-identical to `row_fingerprint_expr`'s own output for every
  dialect — makes the defect unrepeatable rather than just fixed),
  `sidecar_diff_inherits_the_callers_dialect` (proves the fix reaches the composed
  `emit_fingerprint_sidecar_diff`, not only the leaf).

**Decisions:**
- Test 2's negative assertions were narrowed after first-run failure: the per-column
  inner hash (`column_fingerprint_expr`) always emits lowercase `sha256(...)` regardless
  of dialect (GoogleSQL function names are case-insensitive, so this is valid, existing,
  pre-phase-1 behavior in `row_fingerprint_expr` itself) — the test now only pins the
  digest expression's own outer wrapping (`TO_HEX(SHA256(...)`) plus the exact nested
  shape, not a blanket "no lowercase sha256 anywhere" claim.
- Reachability verdict recorded in the outcome's Decision log: **not reachable today**.
  `supports_fingerprint_sidecar` is `true` only for DuckDB in
  `crates/smelt-dialect/src/dialect.rs`; all four runtime entry points in
  `crates/smelt-runtime/src/maintenance_driver/sidecar.rs` refuse with
  `BackendError::unsupported` before calling the emitter on any other backend. The bug
  was latent, not live-hit — fixed anyway per criterion 1 (unconditional).

**For the next planner:**
- `key_expr_for_columns` (delta_key, hardcoded `CAST(... AS VARCHAR)`) and
  `emit_repair_group_digest_select` (hardcoded `bit_xor(hash(sha256(...)))` +
  `CAST(... AS VARCHAR)`) are the remaining dialect-blind fingerprint SQL — already
  scoped as outcome phase 2, not touched here.
- No other dialect-blind emitters were spotted while in this file beyond what phase 2
  already lists.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  full workspace test, example_diagnostics).
- `cargo test -p smelt-logical --quiet` — all passing (includes the 5 new tests).
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 41 passed.
- `cargo test -p smelt-runtime --test fingerprint_sidecar --quiet` — 17 passed.
- No BigQuery live leg run — not needed; all assertions are over emitted SQL text.
