# Phase 2a plan — De-flake the `smelt-core` baseline scratch-hygiene test

## Objective

Make `bash .claude/scripts/verify-phase.sh` unambiguously green so every later phase's
verification means something (success criterion 10). The one red test,
`checkout_scratch_is_deleted_when_materialization_fails`, asserts on a *shared* resource — it
snapshots every `smelt-baseline-*` entry in `std::env::temp_dir()` before and after a failing
`materialize`. Any concurrent `materialize` anywhere on the box perturbs that set. Fix it by
giving `materialize` an explicit scratch-parent seam and pointing the test at a private
directory, so the assertion depends on nothing outside the test.

## Root cause (already established, do not re-derive)

- `crates/smelt-core/tests/baseline/materialize_tests.rs:143,157` snapshot all
  `smelt-baseline-*` entries under `std::env::temp_dir()` and assert set equality.
- In the *same* binary, `materialize_is_not_racing_git_archive_to_a_broken_pipe`
  (`materialize_tests.rs:16`) runs 8 threads × 25 `materialize` calls and is the **only** test
  in the file that does not take `fixtures::lock()` — so it creates and drops
  `smelt-baseline-*` scratch dirs concurrently with the snapshot.
- The race is also cross-process: `smelt-runtime/src/property_diff.rs` and
  `smelt-cli/tests/transformer_metamorphic.rs` call `materialize` too, and cargo runs test
  binaries in parallel. So `lock()` alone cannot fix it — isolation of the scratch parent is
  the only durable fix.

## Spec delta

None. `docs/specs/property_diff.md` §"Baseline materialisation" describes the behaviour
(scratch created first, unwound on every error path); this phase changes no behaviour, only
where the scratch parent comes from. `materialize`'s own signature and semantics are unchanged.

## Tests

Red-green, in `crates/smelt-core/tests/baseline/materialize_tests.rs`:

1. `checkout_scratch_is_deleted_when_materialization_fails` (rewritten) — materialize into a
   test-owned `TempDir` via the new seam; assert that directory is **empty** after the failing
   call. No `std::env::temp_dir()` read, no `lock()` guard needed.
2. `checkout_scratch_is_deleted_on_drop_uses_the_given_parent` (new) — a successful
   `materialize_in` puts its scratch under the supplied parent, and the parent is empty again
   after the checkout drops. Pins the seam's contract in the success direction too.
3. `materialize_defaults_its_scratch_parent_to_the_system_temp_dir` (new) — a successful
   plain `materialize` yields a `project_root()` under `std::env::temp_dir()`. Cheap guard that
   the delegation was not dropped; asserts a path prefix, snapshots nothing.

Red is demonstrated by the *loop* below, not by a single run: the current test passes in
isolation and fails only under concurrency.

## Tasks

1. Reproduce red: run `for i in $(seq 1 20); do cargo test -p smelt-core --test baseline --quiet || break; done`
   and record the failure (the phase-1 and phase-2 summaries both saw it; confirm it here
   before changing anything).
2. In `crates/smelt-core/src/baseline/git.rs`, extract the body of `materialize` into
   `pub fn materialize_in(resolved: &ResolvedBaseline, scratch_parent: &Path) -> Result<BaselineCheckout, BaselineError>`,
   using `tempfile::Builder::new().prefix("smelt-baseline-").tempdir_in(scratch_parent)`.
3. Make `materialize` a one-line delegation to `materialize_in(resolved, &std::env::temp_dir())`;
   keep its existing doc comment and move the "scratch created first, so every error path
   unwinds through `Drop`" paragraph onto `materialize_in` (the function that now owns it).
   Doc-comment `materialize_in` as the explicit-parent seam and name why it exists (a shared
   temp dir is not observable, so a hygiene assertion needs a private parent).
4. Re-export `materialize_in` alongside `materialize` from `crates/smelt-core/src/baseline/mod.rs`
   (match however `materialize` is re-exported).
5. Rewrite test 1 and add tests 2 and 3 per the list above.
6. Leave `materialize_is_not_racing_git_archive_to_a_broken_pipe` untouched — it is a legitimate
   concurrency test and no longer collides with anything.
7. Check no other test in the workspace snapshots `std::env::temp_dir()`
   (`rg -n 'temp_dir\(\)' crates/ --glob '!target'` — at planning time only this one did); if a
   new one has appeared, note it in the summary rather than widening scope.

## Verification

- `for i in $(seq 1 20); do cargo test -p smelt-core --test baseline --quiet || { echo FAIL $i; break; }; done`
  — 20/20 green (the same loop that showed red in task 1).
- `cargo test -p smelt-core --quiet` — green.
- `bash .claude/scripts/verify-phase.sh` — green with **no** exceptions; this is the phase's
  actual deliverable. If anything else is red, it is a new finding: record it in the summary and
  say plainly that the phase's goal was not met.
- `cargo test -p smelt-runtime --lib property_diff --quiet` and
  `cargo test -p smelt-cli --test transformer_metamorphic --quiet` — the other `materialize`
  callers still compile and pass.

## Commit message

`fix(smelt-core): give baseline materialize an explicit scratch parent so hygiene assertions don't race /tmp`
