# Phase 5 plan — validate + close out

## Objective

Close the outcome by discharging criterion 5: delete the now-stale `docs/TODO.md`
§"docs-site sync" bullet, run a `/smelt:validate incremental_models` pass **scoped to the
§Surface "CLI" section** (spec lines 417–541) plus the spec's §References User-docs block, fix
any drift it finds, and land the report as a committed artifact so the close-out judgement in
step 7 of the next plan step has evidence to read. Also confirms criteria 1–4 still hold by
running the standing gates the earlier phases installed.

## Spec delta

No user-visible behaviour changes, so no normative edit is planned up front. Two mechanical
spec edits are in scope **if and only if** the validation finds them drifted:

- `docs/specs/incremental_models.md` §References → **User docs** — the block lists
  `docs-site/docs/guide/{incremental-models,sql-models,materializations}.md` but not
  `docs-site/docs/guide/migrations.md`, the page phase 4 renamed. Add it if the CLI surface
  genuinely references it.
- `docs/specs/incremental_models.md` front-matter `last_reviewed:` — bump from `2026-09-03` to
  `2026-09-05` only if the validation pass reports the CLI surface clean.

Any *behavioural* drift the pass finds is recorded in the report and, if it cannot be fixed
mechanically, added to §Known Divergences with a one-line description — not silently dropped.

## Tests

Red-green does not apply to a validation phase; the phase's oracle is the standing gate set the
earlier phases installed. One new assertion is in scope:

- `docs_front_door::spec_user_docs_block_lists_existing_pages` — every `docs-site/docs/...md`
  path named in `docs/specs/incremental_models.md` §References must resolve to a real file, so a
  future page rename cannot leave the spec's References block pointing at a deleted path (the
  exact rot phase 4's rename would have caused). Confirm red by temporarily pointing one path at
  `guide/backbuild-synthesis.md`.

## Tasks

1. Delete `docs/TODO.md` lines 35–39 (the §"docs-site sync" bullet). Leave the adjacent
   `/smelt:validate` baseline bullet and the `smelt migrate`/`rebuild` wiring bullet alone —
   both name work this outcome lists under "Out of scope".
2. Read `docs/specs/incremental_models.md` §Surface "CLI" (lines 417–541) end to end. For each
   flag, subcommand, printed field and JSON key it states, confirm (a) it exists in
   `crates/smelt-cli/src/` and (b) `docs-site/docs/reference/cli.md` documents it consistently.
3. Read the spec's §References → **User docs** block; confirm every path resolves and that the
   pages it names still describe what it claims (in particular the delta-signature front door
   `guide/incremental-models.md` now leads with).
4. Fix any mechanical drift found in steps 2–3 (missing/renamed path, missing flag in
   `reference/cli.md`, stale printed-field description). Record anything not mechanically
   fixable in the report and, if it is a spec-vs-code gap, as a §Known Divergences bullet.
5. Add the `spec_user_docs_block_lists_existing_pages` test to
   `crates/smelt-cli/tests/docs_front_door.rs`, reusing that file's existing repo-root and
   link-resolution helpers. Confirm red, then green.
6. Bump `last_reviewed:` to `2026-09-05` if step 2–3 came out clean.
7. Write `docs/validations/2026-09-05-incremental_models-cli-surface.md` — scope statement
   (CLI surface + References only, and why: the outcome's criterion 5 asks for the CLI surface),
   a per-item ✅/⚠️ table, the gate results, and an explicit list of what was *not* validated.
8. Write `docs/outcomes/20260904-delta-signature-front-door/phases/05-summary.md` covering all
   five success criteria with the evidence for each, so the next plan step can judge closure.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-cli --test docs_front_door` — now 6 tests (5 existing + the new one).
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage`
  — criteria 1, 2 and 4 still hold.
- `cargo test -p smelt-logical --test backbuild_docs` — criterion 3's doc-sync path still holds.
- `rg -n 'docs-site sync' docs/TODO.md` — empty.
- `rg -in 'four.corners' docs-site/docs/` — empty.

## Commit message

`outcome(delta-signature-front-door): validate the incremental_models CLI surface and close out`
