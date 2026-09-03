# Phase 31 summary — validate + close out (extended)

## Shipped

- `phases/31-validate-incremental_models.md`, `phases/31-validate-incremental_shapes.md` —
  drift reports auditing every Known Divergences bullet criteria 10–19 name against the
  current spec text.
- `docs/specs/incremental_shapes.md`: `last_reviewed` corrected `2026-08-16` → `2026-09-03`
  (phase 29 edited the file's Known Divergences without bumping the date).
- `docs/outcomes/20260815-definition-delta-migrate/outcome.md`: row 31 flipped to `done`; two
  new rows (32, 33) queue the audit's orphaned findings; Decision log entry added.

## Decisions

- Both `/smelt:validate` passes found **zero wording drift** — every criteria-10–19 bullet is
  already either removed or correctly narrowed to its stated residue in the live spec text.
  Phases 10–29 kept their spec edits honest as they went; this phase is confirmation, not
  remediation.
- Two pre-existing bullets (`retain_departed` posture-derived key departure;
  `Override-ladder reach (Open Question)`) map to no live sibling outcome, Out-of-scope entry,
  or Future Extensions item. Per the plan's task 5, added as new phase rows (32, 33) instead
  of fixing or silently dropping them here — 32 is decided-but-unimplemented (has a decision
  record), 33 still needs a product call on whether the first-build-vs-steady-state rule
  should reach the keyed-fold suppression consumer.
- `model_properties.md`'s surviving `INTERSECT`/`EXCEPT` bullet (filter-distribution
  classification) is a genuinely different, still-open residue from the one criterion 16
  closed (per-arm mutation-sensitivity combination) — the spec's own text already
  distinguishes them, so no edit was needed there.

## For the next planner

- Phases 32 and 33 are queued (table rows only, no plan docs yet) — write their
  `phases/32-plan.md` / `phases/33-plan.md` when they become the active row.
- Sibling outcomes `20260815-keyed-grain-residue` and `20260815-partition-grain-residue`
  remain `queued` (not started) — every other surviving Known Divergences bullet in
  `incremental_shapes.md`'s "The key grain"/"The partition grain" sections maps cleanly into
  one of those two, confirmed by this phase's sweep.
- `docs/outcomes/20260815-incremental-spec-closure-confirm` (queued, runs last per the backlog
  order) should re-derive its own audit rather than trust this phase's — its stated job is
  confirming *excluded* bullets are still honestly open, a narrower and complementary check to
  this phase's *closed*-bullet audit.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`)
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed
- `cargo test -p smelt-runtime --test statement_parity` — 32 passed
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed
- `cargo test -p smelt-runtime --test dialect_seam` — 11 passed
- `cargo test -p smelt-runtime --test projection_dialect_invariance` — 4 passed
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed
- `cargo test -p smelt-dialect --test emission_ownership` — 7 passed
- `cargo test -p smelt-core --test hardening_budget` — 4 passed (includes the gate's own
  self-test fixture printing an expected "REGRESSION" line against a synthetic probe crate —
  not a real regression)
- `cargo test -p smelt-types --test unknown_census` — 4 passed
- `cargo test -p smelt-db --test integration registry_consistency` — 6 passed
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed
- Both `/smelt:validate` reports committed under `phases/`; every reported item either already
  closed or recorded as a new phase row (32, 33).
