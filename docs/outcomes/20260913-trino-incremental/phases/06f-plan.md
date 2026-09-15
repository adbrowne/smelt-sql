# Phase 6f — the staged-candidate conditional write's honest landing on Trino

## Objective

Settle phase 6d's residue so criterion 5 does not leave the outcome on a blocked row. 6d proved
`staged_candidate_conditional_parity_on_trino`'s fixture can never exercise the staged-candidate
write through `execute_project` on Trino; this phase takes option (c) — retarget that test to the
route that actually runs, and prove the staged-candidate emitter's Trino statements against the live
engine through a direct harness instead. It also discharges 6d's second finding (the unverified
`column_scoped_cell` dispatch, same shape as the double-dispatch bug 6d fixed) and clears the
unrelated `staged_relation_atomicity` census regression that keeps `verify-phase.sh` red for this
row and every row after it (criterion 11, and the only thing keeping 6e blocked).

**Option (a) is rejected with a measured reason, to be recorded in the decision log:**
`resolve_keyed_fold_state_downgrade`/`resolve_repair_state_downgrade` are flat
`derive_resolved(...) -> find_map` functions, while the override ladder lives inside
`resolve_live_membership_recompute_cell` as ~100 lines of `matching_cell` plumbing with fail-loud
pin validation (`live_cells.rs:126-200`). Threading overrides through both downgrade resolvers means
extracting and re-homing that ladder, and it changes pin semantics on all four backends to make one
Trino test reachable. Option (b) is impossible: the module doc on
`technique_lowering/keyed_membership_recompute_e2e.rs` shows the only body shape that reaches a live
membership cell is a fold aggregate, which is necessarily `KeyedFold`-eligible.

## Spec delta (first)

`docs/specs/multi_backend.md` §"Incremental & schema evolution per backend" — add to the Trino row/
notes: on a backend realising no reconciliation ledger, a membership-sensitive `grain: key` model's
staged-candidate conditional recompute is **subsumed** by the same model's keyed-fold whole-target
rebuild downgrade, which recomputes everything the narrower recompute would have patched. The
family's emitter is still Trino-correct and its statements execute there; it is simply never the
route a keyed model takes on a structure-less backend. One sentence, stated timelessly (no phase
vocabulary), with the same wording applying to Spark's column.

## Tests (red → green)

1. `smelt-runtime --test staged_relation_atomicity::every_production_derivation_site_reads_the_capability`
   — currently RED on `src/cumulative/tests.rs`; green once the census skips files a sibling module
   declares under `#[cfg(test)] mod <name>;`.
2. `staged_relation_atomicity::census_still_flags_a_production_file` (new) — over a synthetic temp
   tree, a genuine non-test-module file containing `StagedRelation::session_temporary(` is still
   reported. Proves the scope narrowing did not blind the gate.
3. `statement_parity::trino::membership_model_whole_target_rebuild_parity_on_trino` (retarget of
   `staged_candidate_conditional_parity_on_trino`) — run 2 executes **exactly one** recorded group
   and it is byte-identical to the whole-target rebuild's own emitter output. Live regression guard
   for 6d's double-dispatch fix.
4. `statement_parity::trino::staged_candidate_conditional_emitter_executes_on_trino` (new) — the
   statements `emit_staged_candidate_conditional_recompute(..., MaintenanceDialect::Trino)` produces
   are executed in order against a real `TrinoBackend` over a seeded target + candidate select, and
   leave the target row- and column-equal to a direct recompute oracle. This is criterion 5's
   surviving proof for this family: the emitter owns the statements and Trino runs them.
5. `availability_seam::column_scoped_cell_is_suppressed_by_a_whole_target_rebuild` (new, offline) —
   for a model whose `NewData` cell carries a whole-target-rebuild downgrade, the column-scoped
   merge must not ALSO dispatch (`execute/project/mod.rs` ~2222). RED first if the bug is real; if it
   is genuinely unreachable (no model can carry both), the test instead asserts that and the doc
   comment records the measured reason.
6. `smelt-cli --test trino_incremental_families` (live) — all existing tests still green,
   `column_scoped_merge_downgrade`'s target still oracle-equal.

## Tasks

1. Fix the `staged_relation_atomicity` census: pre-scan `src/**/*.rs` for
   `#[cfg(test)]` + `mod <ident>;` declarations, resolve `<dir>/<ident>.rs` and `<dir>/<ident>/mod.rs`
   into an exclusion set, and skip those files. Add test 2. Commit note must carry a reviewer
   sign-off line (gate scope change, per CLAUDE.md §Fail-loud discipline).
2. Re-run `verify-phase.sh`; confirm green, and flip phase **6e**'s row to `done` in `outcome.md`
   (its own work was complete — only this gate kept it blocked).
3. Write the spec delta above.
4. Retarget test 3: rename the function, rewrite its assertions against the whole-target rebuild
   emitter, and replace its doc comment with the measured reason the staged-candidate route is
   unreachable here (cite 6d's finding, not this plan).
5. Add test 4 — a direct-emitter live harness in `statement_parity/trino.rs`, using the existing
   `common::trino_backend` / `trino_schema` / `drop_trino_schema` helpers and skipping (with the
   established `eprintln!`) when `SMELT_TRINO_URL` is unset.
6. Add test 5. If it goes red, fix the dispatch in `execute/project/mod.rs` the same way 6d fixed
   membership recompute (`&& whole_target_rebuild_downgrade.is_none()`), and say so in the summary.
7. Append the decision-log entry (option (c) chosen, option (a)'s measured cost, criterion 5's
   surviving shape for this family) and write `phases/06f-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be fully green, including the full-workspace
  `cargo test` (this is the phase that makes it so).
- `cargo test -p smelt-runtime --test staged_relation_atomicity --test execute_parity --quiet`
- Live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`):
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` (46/46 Trino legs) and
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` (18/18).
  Tear down with `bash scripts/trino-down.sh`. If the coordinator is unreachable, emit
  `<<PHASE_BLOCKED>>` — never skip green.
- No ratchet lowered; `.claude/large-file-baseline.txt` updated only if a touched file grew.

## Commit message

`fix(trino): land the staged-candidate write's honest route and unblock the staged-relation census`
