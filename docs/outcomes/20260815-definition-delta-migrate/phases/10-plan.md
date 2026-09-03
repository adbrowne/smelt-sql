# Phase 10 — plan

## Objective

Close out the `definition_deltas.md` half of this outcome: run `/smelt:validate
definition_deltas` and fix (or honestly re-word) every drift it reports, confirm every Known
Divergences bullet phases 1–9 claim to close is actually gone from `definition_deltas.md` **and**
from the sibling files named in success criterion 8, and prove the whole thing with a full
standing-gate sweep. Advances success criteria 8 and 9. This phase changes docs/specs only where
validate finds drift; it is not a feature phase.

## Spec delta

None planned up front — this phase *reacts* to drift. Any edit it makes is a correction to
`docs/specs/definition_deltas.md` (or a sibling spec) so the spec matches the shipped surface, plus
these two already-known corrections:

- `definition_deltas.md` §Known Divergences — the surviving "Resume is approval-marker-based"
  bullet cites "phase 11 (Per-cell frontier addressing)"; the 2026-08-30 renumber moved that work
  to **phase 12**. Fix the pointer.
- `definition_deltas.md` frontmatter `last_reviewed:` → the date this phase lands.

If validate reports a *behaviour* gap rather than a wording gap, do not implement it here: record
it as an accurately-worded Known Divergences bullet pointing at the owning phase row (11–21) and
say so in the summary.

## Tests

Close-out phases assert with greps and gates rather than new unit tests. Add exactly one durable
guard, red-green:

- `crates/smelt-cli/tests/rebuild_dry_run.rs::no_stale_backbuild_verb_in_docs` (or a sibling
  `#[test]` in the same file) — greps `docs/specs/` and `docs-site/docs/` for the literal
  `smelt backbuild` and for `MaintenanceSkeletonColumnAdded`, asserting zero hits. Red first by
  temporarily reintroducing one occurrence. Rationale: criterion 8's rename sweep currently has no
  standing gate, so it can silently regress.

## Tasks

1. Run `/smelt:validate definition_deltas` end to end; capture the drift report verbatim into
   `phases/10-summary.md`.
2. Sweep the criterion-8 sibling list and confirm each is already correct (phases 4/7/8 landed
   them): `docs/specs/cli.md` (verb table + `--dry-run` + cross-ref), `model_selection.md` (line
   ~54), `architecture.md` (walk_coverage path + emitter-parity prose — verb vs module-path
   resolved per bullet), `models.md` (~244, ~346), `seeds.md` (~180). Fix any residue found.
3. Check the residue routed here on 2026-08-30: `docs-site/docs/models.md` and
   `docs-site/docs/seeds.md` (the user-doc pages, not the specs) for stale "no `smelt migrate`
   command" wording; correct or confirm clean.
4. Confirm `crates/smelt-cli/tests/rebuild_dry_run.rs`'s remaining `smelt backbuild` mentions are
   the intentional negative test (the verb must *not* exist) and leave them; scope the new grep
   guard to `docs/` trees so it does not fire on that test.
5. Add the grep guard test from §Tests (red, then green).
6. Fix the stale phase-12 pointer and bump `last_reviewed` in `definition_deltas.md`.
7. For every remaining `definition_deltas.md` Known Divergences bullet, verify it names a
   still-real gap and points at a live owner (an outcome phase row, not a `done` outcome).
8. Run the standing-gate sweep in §Verification; record each result in the summary.
9. Write `phases/10-summary.md` (shipped / decisions / for-the-next-planner / gates).

## Verification

- `bash .claude/scripts/verify-phase.sh` — fmt + clippy (both feature sets) + full `cargo test` +
  `example_diagnostics`.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — the definition-edit
  step kind phase 5 added.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — single-owner
  emission + run-pipeline parity.
- `cargo test -p smelt-logical --test walk_coverage` — property-composition walk.
- `cargo test -p smelt-db --test maintenance_diagnostics` and
  `cargo test -p smelt-cli --test rebuild_dry_run --features duckdb` — phase 7/9 surfaces plus the
  new grep guard.
- `cargo test -p smelt-cli --test e2e --features duckdb` — the suite phase 9's regression turned up.

## Commit message

`docs(definition-deltas): close out the migrate/rebuild wiring — validate sweep + rename guard`
