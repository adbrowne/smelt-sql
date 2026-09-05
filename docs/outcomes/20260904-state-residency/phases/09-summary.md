# Phase 09 summary — conformance-gate leg: `.smelt/` deletion between run steps

**Shipped:**
- `StateDeletion` enum (`Retain` default / `BetweenRuns`) + `deletions`/`nonempty_deletions`
  `AtomicUsize` counters on `LinkCProject`, a `with_state_deletion` builder, and
  `deletions_observed()`/`nonempty_deletions_observed()` accessors
  (`crates/smelt-maintenance-testkit/src/link_c_harness.rs`).
- `LinkCProject::run` — the single seam every family's run goes through — removes
  `project_dir/.smelt` before calling `execute_project` when `state_deletion ==
  BetweenRuns`, recording whether the removed directory existed and was non-empty.
- `crates/smelt-cli/tests/maintenance_conformance/state_deletion.rs` (new, registered in
  `main.rs`): `partition_pool_upholds_equivalence_with_state_deleted`,
  `keyed_pool_upholds_end_state_equivalence_with_state_deleted` (the criterion-1 proof — the
  ledger's never-fold-twice check runs against `_smelt_ledger`), and
  `deletion_leg_is_not_vacuous` (anti-vacuity on the deletion counters).
- `smelt-maintenance-testkit`-local unit test
  `link_c_harness::tests::state_deletion_removes_a_populated_state_dir_before_each_run`.
- `docs/specs/state.md`: the "Conformance gate leg for state deletion" Future Extension bullet
  replaced with a "Downgrade-forcing conformance leg" residual; §References → Tests now names
  the built gate and its `state_deletion.rs` leg.
- `crates/smelt-cli/tests/bakeoff_seam.rs::scratch_project` updated for the new private
  counter fields (`LinkCProject::load(..).with_config(..)` instead of a bare struct literal);
  a `with_config` builder was added alongside `with_state_deletion`.

**Decisions:**
- Deletion happens **before** each run, not after — keeps every post-run manifest read-back in
  the harness working, per the plan.
- Case counts default to 3 per pool (`SMELT_STATE_DELETION_CASES` override) since this leg
  drives a real `execute_project` call AND a filesystem delete per step; a `SMELT_STATE_
  DELETION_CASES=8` local sweep confirmed the pass is not sample luck.
- No family needed `.smelt/` continuity to stay equivalent (task 6's contingency) — every
  admitted case in both the partition and keyed pools upheld its equivalence oracle with
  `.smelt/` deleted before every run. No Blocked entry was needed.

**For the next planner:**
- Phase 9's contingency (fixing a family that turns out to need `.smelt/` continuity) did not
  trigger — nothing outstanding from this phase.
- The residual "Downgrade-forcing conformance leg" Future Extension is explicitly deferred, not
  a phase-10/11 task — it forces an availability downgrade mid-schedule, which is a different
  leg than this phase built.
- Phases 10 (close keyed-grain residue outcome) and 11 (final validate) are next.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 78 passed.
- `SMELT_STATE_DELETION_CASES=8 cargo test -p smelt-cli --test maintenance_conformance --features duckdb state_deletion` — 3 passed.
- `cargo test -p smelt-maintenance-testkit` — 57 passed.
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity` — 37 passed.
