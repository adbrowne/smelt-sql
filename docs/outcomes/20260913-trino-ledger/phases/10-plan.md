# Phase 10 plan — The keyless staged emitter's sentinel takes the residence treatment

## Objective

`emit_staged_candidate_conditional_keyless` is the one staged emitter phase 7 left behind: both
of its relations (the staged candidate and the `__smelt_sentinel_` diff marker) are still
hardcoded `CREATE TEMP TABLE` / `DROP TABLE` with a hardcoded `transactional: true`. Give it
phase 7's `StagedRelation` treatment — residence-derived `CREATE`, reclaim-before-create,
atomicity-derived `transactional` — for **both** relations, and add the standing tests that make
"no Trino path emits a temp table" an asserted fact rather than a comment. Advances criteria 3
(a claim implies a builder) and 8 (the staged relation group without temp tables).

## Spec delta

None. `docs/specs/model_transforms.md` §"The staged-candidate conditional DELETE+INSERT" and
`docs/specs/multi_backend.md` §"Column-scoped merge and conditional-write capabilities" already
state the residence/atomicity rule for the family as a whole (phase 7); this phase brings the
last emitter into conformance with what is already written. If the module doc comment in
`emit/staged.rs` still describes step 1 as `CREATE TEMP TABLE`, that wording is corrected as part
of the code change.

## Tests

Red-green, in this order:

1. `smelt-logical` `emit/staged.rs` unit — `keyless_session_temporary_residence_is_byte_unchanged`:
   the existing DuckDB-shaped call (both relations `session_temporary`, atomic) emits exactly the
   same 7 statements it emits today, byte for byte, `transactional == true`. Guards the migration.
2. `smelt-logical` `emit/staged.rs` unit — `keyless_target_schema_residence_emits_real_tables`:
   with both relations `TargetSchema`/non-atomic, statement 1 and the sentinel `CREATE` use
   `CREATE TABLE` (no `TEMP`), each `CREATE` is preceded by its own `DROP TABLE IF EXISTS`
   reclaim, and both `DROP`s still close the group.
3. `smelt-logical` `emit/staged.rs` unit — `keyless_group_is_not_transactional_when_not_atomic`:
   a non-atomic staged relation yields `transactional == false`.
4. `smelt-runtime` `tests/staged_relation_atomicity.rs` — extend
   `no_non_atomic_backend_is_handed_a_transactional_group` to a fifth emitter (keyless), so the
   standing gate covers every staged emitter rather than four of five.
5. `smelt-runtime` `tests/staged_relation_atomicity.rs` — new
   `no_staged_emitter_hardcodes_a_temp_table_spelling`: a structural assertion over the
   `crates/smelt-logical/src/maintenance/emit/` sources that the literal `CREATE TEMP TABLE`
   appears only in `staged_relation.rs`'s `create_prefix` (test modules excluded by reading only
   the non-`#[cfg(test)]` prefix of each file, or by filename allowlist + an explicit comment).
   This is the regression gate that makes phase 7's rule hold for emitters not yet written.
6. `smelt-runtime` `tests/staged_relation_atomicity.rs` — new
   `trino_cannot_reach_the_keyless_executor_without_a_maintenance_dialect`:
   `smelt_backend::maintenance_dialect(SqlDialect::Trino)` is an `Err` naming the backend, so
   `execute_staged_keyless_recompute`'s first fallible step refuses by name — the asserted form of
   phase 7's "no production Trino caller reaches it".

## Tasks

1. Re-key `emit_staged_candidate_conditional_keyless` to take `staged_relation: &StagedRelation`
   and `sentinel_relation: &StagedRelation`.
2. Derive each `CREATE` from that relation's `create_prefix()`; prepend each relation's
   `reclaim_statement()` when `Some`; keep `drop_statement()` for both trailing `DROP`s.
3. Set `transactional` to `staged_relation.atomic && sentinel_relation.atomic` (a group is atomic
   only if every relation it owns is), documented in the fn doc comment.
4. Update the doc comment's numbered statement list to describe residence-derived spellings, not
   `CREATE TEMP TABLE`, and note the non-atomic recovery obligation the way the four re-keyed
   siblings do.
5. Update the caller `crates/smelt-runtime/src/maintenance_driver/membership/execute.rs`
   (`execute_staged_keyless_recompute`) to build both relations via
   `StagedRelation::session_temporary(format!("__smelt_staged_{table}"))` /
   `..._sentinel_{table}` — names stay byte-identical for DuckDB.
6. Update `crates/smelt-runtime/tests/statement_parity/repair_and_key_addressed.rs`'s direct
   emitter call to the new signature (same session-temporary names, so the parity comparison is
   unchanged).
7. Update the existing keyless unit tests in `emit/staged.rs` to the new signature; add tests 1-3.
8. Add tests 4-6 to `tests/staged_relation_atomicity.rs`, extending its module header to state
   that the gate now covers all five emitters plus the no-hardcoded-temp-table rule.
9. Check `.claude/large-file-baseline.txt` — hand-edit only the affected line(s) if a file grows.
   **Never** run `large-file-check.sh --update` (it drops prior sign-off comments, per phase 7).

## Verification

- `bash .claude/scripts/verify-phase.sh` — the standing gate (fmt, clippy both feature sets,
  shellcheck, full workspace `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --lib maintenance::emit::` — the emitter unit suite.
- `cargo test -p smelt-runtime --test staged_relation_atomicity --test statement_parity` — the
  atomicity gate and per-family executed-vs-emitted parity.
- `cargo test -p smelt-cli --test maintenance_conformance` — DuckDB end-state must be unchanged
  (the byte-stability claim in task 5).
- `bash .claude/scripts/large-file-check.sh` — OK, with no baseline number raised unless a task-9
  hand edit is recorded in the commit.
- No live Trino tier is required for this phase; nothing here executes against a coordinator.

## Commit message

`feat(state): the keyless staged emitter's sentinel takes residence and atomicity as data, with a standing no-temp-table gate`
