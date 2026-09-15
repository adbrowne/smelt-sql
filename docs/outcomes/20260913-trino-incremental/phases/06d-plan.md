# Phase 6d plan — the live-Trino conditional-write parity failure

## Objective

`statement_parity::trino::staged_candidate_conditional_parity_on_trino` fails live and
consistently: run 2 records **2** statement groups instead of 1, the extra one a first-run
bootstrap `CREATE TABLE … AS`, as if run 1's target table were invisible. That is criterion 5's
per-family executed-vs-emitted parity for the merge-less conditional write, so it cannot be
deferred. Measure the cause before changing anything, then either fix the product defect or —
if the cause is the test's own two-backend staging — restructure the test and record the measured
reason.

## Spec delta

None anticipated: nothing here changes user-visible feature surface. Two conditional exceptions,
each only if the measurement demands it:
- If first-run detection on Trino is genuinely defective, add the measured behaviour to
  `docs/specs/multi_backend.md` §"Incremental & schema evolution per backend" (Trino rows) first.
- If the masked-error fix below changes when a run refuses, the new diagnostic gets a
  `docs/specs/diagnostics.md` catalogue entry and a fixture.

## Leading hypothesis (to be confirmed or killed by measurement, not assumed)

The recorded create group can only come from `maintenance_driver/driver.rs:359`'s
`if !table_exists` arm (`execute/project/mod.rs:1781`'s whole-target-rebuild arm is mutually
exclusive with the membership recompute, so it cannot add a second group alongside it). That arm
reads `backend.table_exists(schema, table).await.unwrap_or(false)` — a **masked error**: any
`BackendError` from the live coordinator silently becomes "the target does not exist", which then
re-creates an existing maintained table from one step's delta. Five sites in `smelt-runtime` share
this shape (`maintenance_driver/driver.rs:352`, `maintenance_driver/succession/execute.rs:173`
and `:432`, `cumulative.rs:699`, `execute/project/mod.rs:1545`). Removing the mask is both the
diagnosis instrument and, independently, a fail-loud-discipline fix worth landing either way.

## Tests

1. `smelt-runtime` `maintenance_driver` unit test — `first_run_check_propagates_a_backend_error`:
   a mock backend whose `table_exists` returns `Err` makes `run_windowed_keyed_maintenance` fail,
   never emit a first-run `CREATE TABLE … AS` over a target it could not check. Red today.
2. `smelt-runtime` offline census — `no_table_exists_call_swallows_its_error`: a source scan over
   `crates/smelt-runtime/src/` finds no `table_exists(…).await.unwrap_or(` site, so the class
   cannot regress. Red today (five sites).
3. Live — `staged_candidate_conditional_parity_on_trino` (existing, currently red): green, with
   the recompute group byte-identical to a direct
   `emit_staged_candidate_conditional_recompute` call.
4. Live, only if task 4's branch needs it — `target_table_is_visible_to_a_fresh_backend_
   after_the_creation_run`: after run 1, a freshly constructed `TrinoBackend` reports
   `table_exists(schema, "user_lifetime_status") == true`. Separates "run 1 never created it" from
   "run 1 created it and run 2 cannot see it".

## Tasks

1. Bring the tier up (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`) and reproduce:
   `cargo test -p smelt-runtime --test statement_parity trino::staged_candidate_conditional_parity_on_trino -- --test-threads=1 --nocapture`.
   Capture both recorded groups' SQL verbatim into the summary — that text names which emitter
   produced the extra group.
2. Land tests 1 and 2 red, then make all five `table_exists(…).unwrap_or(false)` sites propagate
   their error. Check each backend's `table_exists` returns `Ok(false)` — not `Err` — for a
   *missing schema* before doing so, or the change refuses honest first runs; fix the backend if
   it does not.
3. Re-run the live test with the mask removed. If it now fails with a backend error, that was the
   cause: diagnose the Trino `table_exists` query against the coordinator directly
   (`{catalog}.information_schema.tables` vs `SHOW TABLES`), fix, and quote the measured error.
4. If `table_exists` succeeds and honestly returns `false`, add test 4 and settle the branch by
   querying the live tier between the two runs:
   - run 1 never created the table → find the run-1 path that skipped or dropped it, fix it, or
     restructure the test's staging per this row's allowance;
   - created but not visible to run 2's fresh backend → measure the REST-catalog visibility
     window and fix at the backend seam (never a sleep); if it is inherent, restructure the test
     to one backend instance and record the measured reason.
5. Confirm parity: the recompute group byte-identical to the emitter, the `USING`-less changed-row
   `DELETE` and non-transactional group assertions still holding.
6. Append the measured cause and the fix taken to outcome.md's Decision log; write
   `phases/06d-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test dry_run_statements --lib`
- `cargo test -p smelt-cli --test trino_ci_wiring`
- Live tier: `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` — every
  Trino leg green, including the previously failing one. Tear down with `bash scripts/trino-down.sh`.
- If the coordinator is unreachable, emit `<<PHASE_BLOCKED>>` — never skip green.

## Commit message

`fix(trino): make the first-run existence check fail loud and land the conditional-write parity leg live`
