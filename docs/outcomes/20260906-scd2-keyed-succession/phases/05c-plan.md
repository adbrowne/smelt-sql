# Phase 5c — Rebuild paths and the statement-parity succession family

## Objective

Close the two rebuild clauses of success criterion 5 and the last outstanding clause of
criterion 4: `--full-refresh` (and `smelt rebuild` over a range) must rebuild the tombstone
ledger from `emit_succession_ledger_rebuild_select` **in the same transaction** as the presented
rebuild — today the phase-5b dispatch runs the patch loop regardless of `request.full_refresh`,
so a full refresh never rebuilds either relation — and `statement_parity` must gain a succession
family leg proving *executed* SQL is byte-identical to the emitters' output over a real
`execute_project` run.

## Spec delta (first)

`docs/specs/incremental_shapes.md` §"The succession grain" → §"The tombstone ledger (hidden
state)" → **Lifecycle** (the paragraph at ~line 1125). Today it reads "`smelt repair` over a
range re-derives the ledger rows whose run-axis partition lies in that range". Two facts make
that unimplementable as written: there is no `smelt repair` command (the range-scoped rebuild
surface is `smelt rebuild <model> --event-time-start/--event-time-end`), and the ledger stores
only `(k, t)` — it carries no run-axis column, so a run-axis restriction is not expressible over
it. Rewrite the sentence: a range rebuild (`smelt rebuild`) re-derives the ledger **in full**
from the whole source, in the same transaction as that range's presented rebuild. Add the
one-clause reason: the ledger is a pure function of the whole (`append_only`, retained) source,
so a whole-source re-derive is the same relation as any range-restricted one and is the only
form the `(k, t)` physical shape admits. Also fix the two other `smelt repair` mentions at
~line 1150 and ~line 1304 in the same section to name `smelt rebuild`.

## Tests (red-green)

`crates/smelt-logical/src/maintenance/emit/succession.rs` (unit, DuckDB-proven):
1. `full_rebuild_group_is_transactional_and_replaces_the_ledger` — `emit_succession_full_rebuild`
   returns one `transactional: true` group of exactly presented-CTAS, ledger `DELETE`, ledger
   `INSERT … SELECT`, and the `INSERT`'s select text is byte-equal to
   `emit_succession_ledger_rebuild_select(.., window_predicate: None)`.
2. `full_rebuild_executes_against_duckdb_and_matches_the_oracle` — executed on a real in-memory
   DuckDB over a seeded source, presented rows equal the model SQL at full refresh and the
   ledger equals the rebuild `SELECT`.

`crates/smelt-runtime/src/maintenance_driver/succession/tests.rs`:
3. `full_refresh_rebuilds_presented_and_ledger_from_source` — after two patched windows, a
   `full_refresh: true` run leaves both relations equal to their whole-source definitions.
4. `full_refresh_drops_a_tombstone_whose_source_row_vanished` — proves DELETE-then-INSERT
   semantics, not an append onto stale ledger rows.
5. `failed_ledger_insert_rolls_back_the_presented_rebuild` — a stub backend failing the ledger
   `INSERT` leaves the presented table at its pre-rebuild contents.
6. `range_rebuild_re_derives_the_whole_ledger` — a `smelt rebuild`-shaped run (window given,
   `full_refresh: false`) ends with the ledger equal to the whole-source rebuild `SELECT`.

`crates/smelt-runtime/tests/statement_parity/succession.rs` (new module, registered in `main.rs`):
7. `succession_patch_executed_statements_match_the_emitters` — the event-delta `SELECT`,
   clock-tie probe and patch group recorded by `RecordingBackend` during a real `execute_project`
   run are byte-identical to direct emitter calls with the batch's own inputs.
8. `succession_full_refresh_executed_statements_match_the_emitters` — the rebuild group recorded
   under `full_refresh: true` is byte-identical to `emit_succession_full_rebuild`.
9. `succession_patch_result_equals_full_refresh` — the `multiset_equal` Link-C leg every other
   family in this suite carries.

## Tasks

1. Make the spec edit above (spec-first; no code yet).
2. Add pure `emit_succession_full_rebuild(presented_table, model_select_sql, source_table,
   key_cols, clock_col, pre_filter, delete_flag_expr, dialect) -> StatementGroup` to
   `crates/smelt-logical/src/maintenance/emit/succession.rs`, composing `emit_create_table_as`'s
   spelling for the presented arm and `emit_succession_ledger_rebuild_select` for the ledger arm,
   `transactional: true`; DuckDB-only assert matching `emit_succession_patch`'s precedent.
3. Add `rebuild_succession_state(backend, schema, cell, columns, compiled_sql, retry, reporter,
   run_id)` to `crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`: ensure-DDL
   (tombstone table, bookkeeping) then `execute_write_with_bookkeeping(&ensure, &[], &group)` with
   the phase-2 emitter's group; report the group via `reporter.maintenance_statements` before it
   runs, as the patch path does.
4. In `crates/smelt-runtime/src/execute/project.rs`, inside the existing succession dispatch
   block: when `request.full_refresh || force_full_refresh`, compile the model and call
   `rebuild_succession_state` instead of `execute_succession_maintenance`, returning the same
   `ModelOutcome::Completed` shape (manifest strategy `"succession_full_rebuild"`). Keep the diff
   to that block — the file's own large-file/hardening ratchets are already red.
5. In the same block's patch path, run `rebuild_succession_state`'s ledger arm as the run's first
   step when the run is a range rebuild — i.e. thread a `rebuild_range: bool` (true when
   `request` carries an explicit rebuild window; derive from the existing selector/window fields
   rather than adding a new `ExecuteRequest` field if one already distinguishes them, otherwise
   treat `smelt rebuild` as `full_refresh`-of-the-ledger-only and say so in the doc comment).
6. Add tests 1–6.
7. Add `crates/smelt-runtime/tests/statement_parity/succession.rs` with tests 7–9, reusing
   `RecordingBackend`, `multiset_equal` and the `fixtures/succession/` workspace from phase 5b;
   `mod succession;` in `statement_parity/main.rs`, and extend that file's header comment to name
   the succession family.

## Verification

- `cargo test -p smelt-logical --lib maintenance::emit::succession`
- `cargo test -p smelt-runtime --lib maintenance_driver::succession`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-runtime --test technique_lowering succession`
- `cargo test -p smelt-runtime --test execute_parity`
- `bash .claude/scripts/verify-phase.sh` — expect the same single pre-existing
  `large_file_ratchet::gate_passes_on_committed_tree` failure phase 5b recorded (the loop's
  shrink step owns it); any *other* red gate is this phase's to fix.

## Commit message

`feat(smelt-runtime): rebuild the succession ledger with the presented table on full refresh`
