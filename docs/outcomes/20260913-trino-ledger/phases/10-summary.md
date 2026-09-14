# Phase 10 summary — The keyless staged emitter's sentinel takes the residence treatment

**Shipped:**
- `emit_staged_candidate_conditional_keyless` (`crates/smelt-logical/src/maintenance/emit/
  staged.rs`) re-keyed from `staged_relation: &str, sentinel_relation: &str` to
  `staged_relation: &StagedRelation, sentinel_relation: &StagedRelation` — the same treatment
  phase 7 gave the other four staged emitters, now covering all five. Both relations' `CREATE`
  spellings are derived from `create_prefix()`, each preceded by its own `reclaim_statement()`
  when `Some`, and `transactional` is `staged_relation.atomic && sentinel_relation.atomic` (a
  group is atomic only if every relation it owns is).
- Caller `crates/smelt-runtime/src/maintenance_driver/membership/execute.rs`
  (`execute_staged_keyless_recompute`) updated to build both relations via
  `StagedRelation::session_temporary(...)` — names byte-identical to before (`__smelt_staged_
  {table}`, `__smelt_sentinel_{table}`), so DuckDB/Spark/BigQuery behavior is unchanged.
- `crates/smelt-runtime/tests/statement_parity/repair_and_key_addressed.rs`'s direct emitter
  call updated to the new signature (same names, so the parity comparison is unchanged).
- New unit tests in `emit/staged.rs`: `keyless_session_temporary_residence_is_byte_unchanged`
  (byte-for-byte migration guard, 7 statements), `keyless_target_schema_residence_emits_real_tables`
  (both `CREATE`s use `CREATE TABLE`, each preceded by its own `DROP TABLE IF EXISTS` reclaim),
  `keyless_group_is_not_transactional_when_not_atomic` (non-atomic ⇒ `false`, plus a mixed-atomicity
  case proving any one non-atomic relation makes the whole group non-atomic).
- `crates/smelt-runtime/tests/staged_relation_atomicity.rs` extended: the existing
  `no_non_atomic_backend_is_handed_a_transactional_group` now covers the keyless emitter too (5th
  emitter, both atomic and non-atomic shapes); two new standing tests —
  `no_staged_emitter_hardcodes_a_temp_table_spelling` (structural scan: the string literal
  `"CREATE TEMP TABLE` may appear in exactly `staged_relation.rs`'s `create_prefix()`, scanning
  only each file's pre-`#[cfg(test)]` prefix so doc comments and test-literal SQL don't trip it)
  and `trino_cannot_reach_the_keyless_executor_without_a_maintenance_dialect`
  (`smelt_backend::maintenance_dialect(SqlDialect::Trino)` is an `Err` naming Trino).

**Decisions:**
- The structural no-hardcoded-temp-table scan matches on the string literal `"CREATE TEMP TABLE`
  (leading quote), not the bare phrase — the bare phrase also appears in doc comments describing
  the emitted statement shape (`recompute.rs`'s numbered lists), which are documentation, not an
  emission site, and must not trip the gate.
- Production call sites still construct `StagedRelation::session_temporary(...)` unconditionally
  (no `BackendCapabilities` threading) — matches phase 7's standing decision: no
  `MaintenanceDialect::Trino` variant exists, so no live Trino path reaches
  `execute_staged_keyless_recompute` today; its first fallible step
  (`maintenance_dialect(backend.dialect())?`) already refuses by name before any `StagedRelation`
  is built, which is exactly what the new sixth test asserts.

**For the next planner:**
- Phase 11 (surface and close: `smelt explain` Trino downgrades, diagnostics catalogue,
  `examples/broken/` fixtures, docs-site update) is next, currently `pending`.
- No live Trino tier was needed for this phase — nothing here executes against a coordinator.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — fmt initially failed (two blocks needed rustfmt's
  reflow); fixed with `cargo fmt --all`, then reconfirmed: fmt clean, clippy zero-warnings on
  both feature sets, shellcheck zero-findings, full workspace `cargo test` PASS, `example_diagnostics`
  PASS (the last three confirmed in the pre-fmt-fix run, which the whitespace-only fmt fix cannot
  have affected; fmt/clippy/shellcheck reconfirmed individually after the fix).
- `cargo test -p smelt-logical --lib maintenance::emit::` — 94 passed.
- `cargo test -p smelt-runtime --test staged_relation_atomicity --test statement_parity` — 3 + 41
  passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 104 passed (DuckDB end-state
  unchanged).
- `bash .claude/scripts/large-file-check.sh` — OK, no baseline change needed.
