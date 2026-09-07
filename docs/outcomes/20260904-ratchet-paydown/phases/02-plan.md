# Phase 02 plan — `smelt-db`: convert the added `unwrap` sites

## Objective

Bring `smelt-db unwrap` back to its pre-burst value or below (success criterion 1) and discharge
the three sites `a68a8268` added (success criterion 2) by removing the duplicated
`RwLock::read()/write().unwrap()` + copy-pasted rationale-comment pattern from `Database`'s
input registries in favour of two small poison-recovering accessors. This resolves the
contradiction inside `01-census.md` (its §3 restatement keeps `smelt-db unwrap ≤ 16`; its
"Phase ownership" section says the count "stays 19") in favour of §3 — the honest reading, since
the pattern is mechanically removable without changing any reachable behaviour.

## Spec delta

None. No user-visible feature behaviour changes: the lock is documented as unpoisonable in this
single-threaded Salsa mutation context, so both the old panic and the new recovery are
unreachable in production. `docs/specs/architecture.md` §"Fail-loud discipline" already covers the
ratchet; no edit needed.

## Tests

Add to `crates/smelt-db/src/tests.rs` (test file — excluded from the hardening count):

1. `poisoned_files_registry_does_not_panic` — poison `db.files` from a panicking thread, then
   assert `db.source_file(path)` still returns the registered `SourceFile` (red today: `.unwrap()`
   panics).
2. `poisoned_deployed_schemas_registry_does_not_panic` — same for `db.deployed_schemas` via
   `db.deployed_schema(root, model)`, covering one of the three sites the burst added.
3. `set_source_file_still_upserts_after_poisoning` — after poisoning `db.files`, a second
   `set_source_file` for the same path updates rather than duplicates, proving the write-side
   accessor is on the same recovery path.

Existing `smelt-db` suites (`cargo test -p smelt-db`) are the regression net for the ten
pre-existing sites the refactor also touches.

## Tasks

1. In `crates/smelt-db/src/lib.rs`, add `use std::sync::{RwLockReadGuard, RwLockWriteGuard}` and
   two private free functions next to `Database`:
   `fn read_registry<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T>` and
   `fn write_registry<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T>`, each
   `lock.read()/write().unwrap_or_else(|poisoned| poisoned.into_inner())`.
2. Give the pair ONE doc comment carrying the rationale that is currently copy-pasted at each
   call site: the lock is only poisoned by a panic that cannot occur in the single-threaded Salsa
   mutation context, and recovering the guard keeps the registry readable rather than cascading a
   second panic.
3. Rewrite all 13 call sites in `lib.rs` (lines ~529, 539, 554, 563, 574, 583, 589, 646, 657, 674,
   696, 713, 730) to `read_registry(&self.files)` / `write_registry(&self.projects)` etc., deleting
   the now-redundant per-site `// invariant: same RwLock poisoning rationale as set_source_file.`
   comments.
4. Write the three tests from §Tests; confirm each fails before the refactor and passes after
   (red-green, one at a time).
5. Run `bash .claude/scripts/hardening-budget.sh` and confirm it now reports the smelt-db `unwrap`
   count has FALLEN (two-sided ratchet error), then `--update` the baseline and confirm the row
   reads `smelt-db unwrap 6` (19 − 13; verify the actual number rather than trusting the
   arithmetic) with no other crate/pattern row changed — `git diff .claude/hardening-baseline.txt`
   must show exactly one changed line.
6. Write `phases/02-summary.md`: the final `smelt-db unwrap` number, confirmation that criterion 1's
   `smelt-db` half is met, and a note for the phase-4 planner that the criterion-1 restatement only
   needs to change its `println` clause.

## Verification

- `cargo test -p smelt-db --quiet 2>&1 | tail -40`
- `bash .claude/scripts/hardening-budget.sh` (OK after `--update`)
- `cargo test -p smelt-core --test hardening_budget --quiet`
- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN
- `cargo test -p smelt-runtime --test execute_parity --quiet` (behaviour-unchanged check; the
  registries feed every compile path)

## Commit message

`refactor(db): recover poisoned registry locks via shared accessors — ratchet sign-off: 20260904-ratchet-paydown phase 2`
