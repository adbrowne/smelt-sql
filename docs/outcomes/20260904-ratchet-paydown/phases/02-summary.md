# Phase 02 summary — `smelt-db`: convert the added `unwrap` sites

## Shipped

- `crates/smelt-db/src/lib.rs`: two private poison-recovering accessors,
  `read_registry`/`write_registry`, each `lock.read()/write().unwrap_or_else(|poisoned|
  poisoned.into_inner())`, with one shared doc comment carrying the poisoning rationale.
- All 13 call sites across `set_source_file`, `set_project_input`, `set_project_smelt_yml`,
  `source_file`, `project_input`, `set_loader_file`, `loader_file`, `set_deployed_schema`,
  `deployed_schema` now go through the shared accessors; every per-site
  `// invariant: same RwLock poisoning rationale as set_source_file.` comment removed.
- Three new tests in `crates/smelt-db/src/tests.rs`: `poisoned_files_registry_does_not_panic`,
  `poisoned_deployed_schemas_registry_does_not_panic`,
  `set_source_file_still_upserts_after_poisoning`.
- `.claude/hardening-baseline.txt` regenerated: `smelt-db unwrap 19` → `6` (13 removed), exactly
  one line changed (`git diff` confirmed).

## Decisions

- Poisoning tests clone only `Arc::clone(&test_db.db.files)` (the raw lock), not
  `test_db.db.clone()`. Cloning the whole `Database` stands up a Salsa snapshot; a subsequent
  `&mut self` mutation on the original handle then blocks on Salsa's cancellation machinery
  waiting to observe the snapshot's drop — a genuine deadlock reproduced live in
  `set_source_file_still_upserts_after_poisoning` (hung >30 min under the whole-`Database`-clone
  form; fixed by cloning just the `Arc<RwLock<_>>` field, which never touches Salsa). Recorded
  here since the plan's task list didn't anticipate it.
- No spec edit: confirmed the plan's call — the lock is unpoisonable in the single-threaded Salsa
  mutation context, so this is a pure hardening refactor with no behaviour change.

## For the next planner

- Criterion 1's `smelt-db` half is now met honestly: `unwrap 6` is comfortably under the
  pre-burst target of `≤ 16`. Phase 4's restatement only needs to touch the `println` clause, per
  plan 02's decision log entry — confirmed, nothing else to adjust.
- If a future phase needs the same whole-`Database`-clone-then-mutate pattern anywhere else
  (tests or production), the Salsa snapshot/cancellation deadlock above is worth flagging before
  reuse — it is not specific to this refactor, it's a property of `Database: Clone` plus a live
  `&mut self` call while a clone is still resolving its drop.
- No other gaps surfaced; phase 3 (`smelt-cli` `println!`/`expect` sites) is unaffected by this
  phase's work.

## Gates

- `cargo test -p smelt-db --quiet` — all green (7 + 43 + 184 + 2 + 3 + 89 + 7 tests, incl. the 3
  new poison-recovery tests).
- `bash .claude/scripts/hardening-budget.sh --update` — `smelt-db unwrap` 19 → 6, single-line
  baseline diff.
- `cargo test -p smelt-core --test hardening_budget --quiet` — 4 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, workspace
  tests, example_diagnostics).
- `cargo test -p smelt-runtime --test execute_parity --quiet` — 4 passed.
