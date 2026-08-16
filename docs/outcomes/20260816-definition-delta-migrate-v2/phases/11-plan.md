# Phase 11 plan — validate + close out

## Objective

Run `/smelt:validate definition_deltas` and close every drift it reports, so the spec describes
exactly what phases 1–10 shipped: §References refreshed (code/tests/user-docs), each remaining
Known Divergence bullet re-checked against the code and either deleted (stale) or re-pointed at a
real tracker (this outcome is finishing, so a bullet that survives cannot cite it as "tracked"
without saying it is deliberately unclosed), and `last_reviewed` bumped. Then sweep the full
standing-gate set green. Advances success criterion 8 (and finishes 7's spec-side residue).

## Spec delta

`docs/specs/definition_deltas.md` — bookkeeping only, no user-visible behaviour change:

- **§References → Code**: add `crates/smelt-runtime/src/migrate.rs`,
  `crates/smelt-runtime/src/schema_evolution.rs` (`resolve_definition_change_route`),
  `crates/smelt-cli/src/commands/migrate.rs`, the approval store, and the
  `ProjectInput::deployed_columns` Salsa input + `workspace_ingest::read_deployed_columns`.
- **§References → Tests**: add the migrate CLI tests, the conformance suite's definition-edit
  pool + `MigrateApply` legs, and the docs ratchets (`rebuild_dry_run.rs`).
- **§References → User docs**: replace "none yet — the docs-site page for migration lands with the
  wiring plan" with `docs-site/docs/guide/backbuild-synthesis.md` and
  `docs-site/docs/reference/cli.md` §`smelt migrate`.
- **§References → Plans (history)**: add this outcome directory.
- **§Known Divergences**: re-verify all four bullets against the code. Bullet 1 (live
  definition-change handling is the narrower column-add-only third mechanism) is the one most
  likely changed by phases 7/9 — restate it to what is actually true today, or delete it if
  phase 7's routing closed it. Every surviving bullet's tracking line must say it is deliberately
  out of this outcome's scope (naming the "## Out of scope" entry) rather than implying this
  outcome will close it.
- **Header**: `last_reviewed: 2026-08-17`.

## Tests

No behaviour change, so no new red-green product test. The oracle is the gate set plus one new
standing check:

1. `crates/smelt-logical/tests/backbuild_docs.rs::spec_references_are_live_paths` — every path
   listed under `definition_deltas.md` §References → Code / Tests exists on disk. Red now
   (nothing asserts it); green after the §References refresh, and it keeps the list from rotting.

## Tasks

1. Run `/smelt:validate definition_deltas` (or its checks by hand if the command needs an
   interactive turn); capture the drift list.
2. Add the failing `spec_references_are_live_paths` test (red).
3. Refresh §References (Code, Tests, User docs, Plans) → test green.
4. Re-read each §Known Divergences bullet against the code it describes; delete stale ones, and
   reword survivors to name the "## Out of scope" entry that owns them.
5. Bump `last_reviewed`; sweep the spec body for Timeless-oracle violations (`Phase [A-Z0-9]`,
   plan-phase vocabulary) and for any remaining "not implemented / no CLI command yet" claim that
   phases 1–10 falsified.
6. Re-run `/smelt:validate definition_deltas` and confirm a clean report; paste its verdict into
   the phase summary.
7. Full standing-gate sweep (below); record each result in the phase summary.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20` (includes the
  phase-5 definition-edit pool and phase-6 `MigrateApply` legs)
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test execute_parity --quiet 2>&1 | tail -20`
- `cargo test -p smelt-logical --test walk_coverage --quiet 2>&1 | tail -20`
- `cargo test -p smelt-logical --test backbuild_docs --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test rebuild_dry_run --quiet 2>&1 | tail -20`
- `cd docs-site && uv run mkdocs build --strict` (pre-existing unrelated anchor warnings noted in
  the phase-10 summary are acceptable; no *new* ones)

## Commit message

`docs(definition-deltas): close spec drift after the smelt migrate wiring and sweep standing gates`
