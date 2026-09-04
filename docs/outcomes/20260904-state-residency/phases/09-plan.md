# Phase 09 plan — conformance-gate leg: `.smelt/` deletion between run steps

## Objective

Make the outcome's headline claim executable: a standing generative leg of
`cargo test -p smelt-cli --test maintenance_conformance` deletes the project's
`.smelt/` directory between run steps and asserts the equivalence oracle still
holds. Advances success criterion 6 directly, and is the end-to-end proof of
criterion 1 (the reconciliation ledger's never-fold-twice check survives because
it is engine-resident, not file-resident).

## Spec delta

`docs/specs/state.md` §Future Extensions — delete the **"Conformance gate leg for
state deletion"** bullet (its "sensible only after the ledger's move" precondition
is met and the leg now exists) and, in its place, name the built gate in
§References → Tests alongside the other standing gates. The parenthetical "(and,
later, downgrade-forcing)" half is *not* built here: carry that residual forward as
a one-line Future Extension bullet of its own ("Downgrade-forcing conformance leg").
No user-visible behaviour changes, so this is the only spec edit.

## Tests (red-green)

1. `smelt-maintenance-testkit` unit — `link_c_harness::state_deletion_removes_a_populated_state_dir_before_each_run`:
   with `StateDeletion::BetweenRuns`, a second `run` finds no `.smelt/` left by the
   first; the deletion counter records that the removed directory was non-empty.
2. `maintenance_conformance::state_deletion::partition_pool_upholds_equivalence_with_state_deleted`:
   the append-only partition pool (`arb_recipe` + `arb_schedule_for`, driven through
   the existing `gate::drive_and_assert`) still upholds S-restricted multiset
   equivalence with `.smelt/` removed before every run.
3. `maintenance_conformance::state_deletion::keyed_pool_upholds_end_state_equivalence_with_state_deleted`:
   same for the keyed pool (`arb_keyed_combiner`/`arb_keyed_schedule` +
   `gate::drive_keyed_and_assert`) — the family whose never-fold-twice check runs
   against `_smelt_ledger`, so a green result is the criterion-1 proof.
4. `maintenance_conformance::state_deletion::deletion_leg_is_not_vacuous`:
   anti-vacuity — after a driven schedule the harness's deletion counter is > 0 and
   every counted deletion removed a directory that existed and was non-empty. Locks
   the leg against silently degrading into "delete nothing, assert equivalence".

## Tasks

1. `crates/smelt-maintenance-testkit/src/link_c_harness.rs`: add `pub enum StateDeletion
   { Retain, BetweenRuns }` (default `Retain`), a `state_deletion` field and an
   `Arc<AtomicUsize>` `deletions` counter on `LinkCProject`, and a
   `with_state_deletion(self, mode) -> Self` builder.
2. In `LinkCProject::run` (the single seam every family's run goes through — mirrors
   phase 8's one-seam decision), under `BetweenRuns` remove `project_dir/.smelt`
   *before* calling `execute_project`, recording whether it existed and was non-empty;
   expose `deletions_observed()` / `nonempty_deletions_observed()`. Deleting before
   each run (not after) keeps every post-run manifest read-back in the harness working.
3. Update the one external `LinkCProject { .. }` struct literal
   (`crates/smelt-cli/tests/bakeoff_seam.rs:188`) for the new fields.
4. New `crates/smelt-cli/tests/maintenance_conformance/state_deletion.rs` + `mod
   state_deletion;` in that target's `main.rs`. Reuse the existing public staging and
   drive helpers (`gate::stage_recipe`, `gate::drive_and_assert`,
   `gate::stage_keyed_recipe`, `gate::drive_keyed_and_assert`) — no drive loop is
   duplicated. Deterministic `TestRunner::deterministic()`, default case count small
   (3 per pool) with a `SMELT_STATE_DELETION_CASES` env override, so the standing
   gate's wall-clock cost stays bounded.
5. Red-green: land tests 1-4 failing first where they can fail (test 1 against the
   un-built toggle), then the harness change.
6. If a family turns out to *need* `.smelt/` continuity to stay equivalent (the
   likeliest candidate is a `MigrateModel` step's approval store, which phase 8
   classified `observability`), that is a genuine criterion-1 finding, not a reason to
   narrow the leg: fix the production path or, if the fix is out of this phase's
   reach, record it in the outcome's Blocked section rather than skipping the family.
7. Apply the §Future Extensions / §References spec edit from the Spec delta above.

## Verification

- `bash .claude/scripts/verify-phase.sh` (mandatory).
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb 2>&1 | tail -40`
  — the whole gate, including the new leg.
- `SMELT_STATE_DELETION_CASES=8 cargo test -p smelt-cli --test maintenance_conformance
  --features duckdb state_deletion 2>&1 | tail -20` — a deeper local sweep of the new
  leg only, to confirm it is not passing by sample luck.
- `cargo test -p smelt-maintenance-testkit 2>&1 | tail -20`.
- Unchanged-gate checks: `cargo test -p smelt-runtime --test execute_parity --test
  statement_parity 2>&1 | tail -20`.

## Commit message

`test(state-residency): conformance leg deleting .smelt/ between run steps`
