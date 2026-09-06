# Phase 7b1 plan — Re-run tolerance under deletes

## Objective

Fix the spurious `SuccessionClockTie` phase 7b uncovered: refolding any window that
contains a delete-flagged event fails, because the clock-tie probe's signature compares a
tombstone row's NULLed payload against the *same* event's real payload replayed from the
source. This is a direct failure of criterion 5 ("re-folding a window leaves table and
ledger byte-identical") and blocks criterion 6's `repair.rs` widening in phase 7c, which
re-drives windows after a ledger rebuild.

## Spec delta

None. `docs/specs/incremental_shapes.md` §"Run shape and late events" already states the
correct rule — *"Against a stored tombstone only the delete flag is comparable, since the
ledger carries no row-local content"*, and *"matching … a stored tombstone's `(k, t)` and
delete flag — is always a re-presentation and a no-op"*. The emitter simply does not
implement it. Quote that sentence in the fixed code's doc comment so the tie is anchored.

## The defect (for the implementer)

`emit_succession_clock_tie_probe` (`crates/smelt-logical/src/maintenance/emit/succession.rs`)
builds the per-row tie signature as
`CAST(__smelt_is_delete AS TEXT) || '|' || (COALESCE(CAST(<payload> AS TEXT),'') || …)`.
`build_domain_cte`'s tombstone branch projects `NULL AS <payload>` (a tombstone genuinely
carries no content), so a replayed delete yields two domain rows for one event —
`'true|'` from the ledger and `'true|bronze'` from the batch — and
`COUNT(DISTINCT …) > 1` calls it a content collision.

The fix: a delete row's signature is its flag **alone** — e.g.
`CASE WHEN __smelt_is_delete THEN 'D' ELSE 'I|' || (<payload sig>) END`. Delete-vs-insert at
one `(k, t)` still yields two distinct signatures; two non-identical inserts still do; two
deletes at one `(k, t)` are indistinguishable by construction and are correctly silent.
Scope is the signature expression only — `build_domain_cte`'s NULL projection is right, and
`emit_succession_patch`'s payload asymmetry is harmless (a delete row can never reach the
`WHEN NOT MATCHED AND NOT source.__smelt_is_delete THEN INSERT` arm, and its payload never
reaches the presented table), so do not change either.

## Tests

Unit tests in `crates/smelt-logical/src/maintenance/emit/succession.rs` (each DuckDB-proven,
matching the module's existing probe tests):

1. `clock_tie_probe_is_silent_when_a_tombstoned_delete_is_replayed` — ledger row `(k, t)`
   with NULL payload plus a batch row `(k, t)` with the same event's real payload and the
   delete flag set ⇒ `violation_count = 0`. **The red test.**
2. `clock_tie_probe_still_fires_for_a_delete_and_an_insert_at_one_clock_value` — a presented
   (non-delete) row and a batch delete at one `(k, t)` ⇒ `violation_count = 1`.
3. `clock_tie_probe_still_fires_for_two_non_identical_inserts` — regression guard for the
   existing behaviour (extend the existing collision test rather than duplicating it if it
   already covers this shape).
4. `clock_tie_probe_is_silent_for_two_identical_deletes_at_one_clock_value` — the spec's
   "identical ⇒ re-presentation" rule, now that content is not compared for deletes.

Conformance legs in `crates/smelt-cli/tests/maintenance_conformance/succession.rs`:

5. `repeated_window_application_with_deletes_is_idempotent` — the delete-flagged variant of
   leg 6 that phase 7b had to weaken to a plain recipe: drive a window containing a delete,
   drive the same window again, assert oracle-equivalence and that presented + tombstone
   tables are byte-identical across the refold.
6. `refold_after_a_full_refresh_ledger_rebuild_is_clean` — answers phase 7b's uninvestigated
   question: stage a delete-bearing recipe, drive windows, run `--full-refresh` (rebuilding
   ledger + presented in one transaction), then re-drive the last window; assert no
   `SuccessionClockTie` and oracle-equivalence.

## Tasks

1. Write tests 1–4 red against the current emitter; confirm 1 and 4 fail and 2–3 pass.
2. Replace the `sig_expr`/`HAVING COUNT(DISTINCT …)` construction with the delete-aware
   `CASE` signature; update the function's doc comment with the spec sentence.
3. Green tests 1–4; re-run the module's full unit suite for emitter snapshot drift.
4. Restore leg 6 as test 5 (delete-flagged recipe) in the conformance suite, and delete the
   phase-7b comment that explained the weakening.
5. Add test 6, reusing `gate_succession`'s existing stage/insert/drive helpers; add a
   full-refresh drive helper to `crates/smelt-maintenance-testkit/src/gate_succession.rs`
   only if no existing helper covers it.
6. Append the fixed probe's shape to the phase-7b decision-log note if the summary's
   description of the bug is now stale.

## Verification

- `cargo test -p smelt-logical --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -40` (full
  seeded sample, not just the succession module)
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20` (the probe is
  an emitted statement; executed == emitted must still hold)
- `bash .claude/scripts/verify-phase.sh`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`fix(succession): compare only the delete flag for tombstoned rows in the clock-tie probe`
