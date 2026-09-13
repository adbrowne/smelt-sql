# Phase 9e plan — the ledger-free succession full rebuild

## Objective

Make 9c's dispatch fix actually reach Databricks: a `state_downgraded` succession cell must
*complete* a full rebuild, not hit the same `realises_tombstone_ledger` refusal one function
further down. Advances criteria 6, 7 and 8 — every Databricks run of the whole model set is
blocked on this, and both live sweeps are waiting behind it. Entirely offline: the branch is
keyed on `cell.state_downgraded`, never on the dialect, so a real DuckDB backend can execute
the downgraded path in a test.

## Spec delta

`docs/specs/state.md` §"The degradation contract", extending the succession-grain paragraph
9d added (the one ending "…no way to retract a delete event it never saw a tombstone for"):
state what the downgraded rebuild *writes* and *skips* — it writes the presented table alone
(the same fold the ledger-bearing rebuild's presented arm produces, so the two engines'
presented tables stay row- and column-identical), touches no tombstone relation at all, and
forgoes the clock-tie probe, whose domain CTE reads a ledger this target has none of. A
`(k, t)` tie is therefore resolved by the rebuild fold's own deterministic tie-break instead
of being refused — the second cost the contract trades for correctness on such a backend, and
a fact criterion 9's findings handoff records.

## Tests

1. `emit_succession_full_rebuild_ledgerless_emits_only_the_presented_arm` (smelt-logical,
   `maintenance/emit/succession/tests.rs`) — for the Spark dialect the new emitter returns one
   non-transactional statement whose SQL is byte-identical to the presented statement
   `emit_succession_full_rebuild` produces for the same arguments on DuckDB, and mentions no
   tombstone table name.
2. `emit_succession_full_rebuild_ledgerless_accepts_every_dialect` (smelt-logical) — the new
   emitter has no `check_succession_dialect` refusal: DuckDb, Spark and BigQuery all return a
   group (the whole point — this is the emitter a no-ledger backend uses).
3. `a_downgraded_cell_rebuilds_for_real_against_duckdb` (smelt-runtime, new
   `tests/succession_downgraded_rebuild.rs`) — the executed-not-just-dispatched test 9d asked
   for: build a `SuccessionCell { state_downgraded: true, .. }` over a seeded DuckDB table,
   call `rebuild_succession_state` against a live `DuckDbBackend`, assert it returns `Ok`, the
   presented table holds exactly the folded rows, and **no** tombstone table was created.
4. `a_downgraded_rebuild_reports_no_tombstone_or_probe_statement` (same file) — via a recording
   reporter, the statements reported for the downgraded run are exactly the emitter's one
   statement: no clock-tie probe, no `tombstone_table_ddl`, no ledger `DELETE`/`INSERT`.
5. `a_non_downgraded_cell_on_a_no_ledger_dialect_still_refuses` (smelt-runtime,
   `maintenance_driver/succession/tests.rs` or the new file) — the backstop refusal survives:
   the `realises_tombstone_ledger` bail still fires for a cell that is *not* downgraded, so a
   caller that skipped the availability check is still caught.
6. `a_downgraded_cell_rebuilds_idempotently` (same file) — running the downgraded rebuild twice
   over unchanged source leaves the presented table row-identical (the whole-source recompute
   the degradation contract promises).

## Tasks

1. Spec first: land the §"The degradation contract" paragraph edit above.
2. In `crates/smelt-logical/src/maintenance/emit/succession/mod.rs`, extract the presented arm
   of `emit_succession_full_rebuild` (the `folded_select` construction plus
   `emit_create_table_as`) into a private helper both emitters call, so the two paths cannot
   drift in fold shape or column order.
3. Add `pub fn emit_succession_full_rebuild_ledgerless(...)` beside it: same arguments minus
   `source_table`/`pre_filter`/`delete_flag_expr` (ledger-only inputs), no
   `check_succession_dialect`, returning `StatementGroup { statements: vec![presented], transactional: false }`
   — infallible, so no `Result`. Doc-comment it as the single owner of the downgraded rebuild's
   statement, citing `state.md` §"The degradation contract".
4. Write tests 1-2 (red), then confirm green.
5. In `crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`, split
   `rebuild_succession_state` on `cell.state_downgraded` **before** the
   `realises_tombstone_ledger` guard: the downgraded arm skips the ensure-DDL, the presented
   shell, the clock-tie probe and the ledger statements, drops the presented table (idempotent,
   as today), reports the one statement via `reporter.maintenance_statements`, and executes it
   through the same `retry_backend_call`/`execute_write_with_bookkeeping` seam with empty
   ensure/cleanup lists. The existing guard stays put on the non-downgraded arm as the backstop.
6. Write tests 3-6 (red), then implement to green; keep the `state_guard_census` annotation
   discipline intact (the branch is a plan-derived boolean, not a dialect comparison, so the
   census stays empty).
7. Re-read `crates/smelt-cli/tests/github_activity_dual_target.rs:1125`'s deferral comment and
   update it to name 9f as the phase that restores the report gates (do **not** restore them —
   `08-parity.json` still does not exist).

## Verification

- `cargo test -p smelt-logical --lib maintenance::emit::succession` and `cargo test -p smelt-logical --test succession_emit`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-runtime --test succession_downgraded_rebuild`
- `cargo test -p smelt-runtime --test state_guard_census --test statement_parity --test execute_parity`
- `cargo test -p smelt-runtime --test technique_lowering` (the ledger-bearing succession e2e must
  be untouched — that is the regression this refactor risks)
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`fix(succession): rebuild a state-downgraded cell without a tombstone ledger`
