# Phase 4 plan — run all four `/smelt:validate` invocations; fix any drift

## Objective

Produce a full-spec drift report for each of the four anchor specs (`definition_deltas`,
`incremental_models`, `incremental_shapes`, `model_properties`) by executing the
`/smelt:validate` process, and resolve every drift item found. Advances success criterion 4
directly, and supplies criterion 1's report with per-spec evidence that the surviving open
bullets are the only unresolved items.

## Spec delta

None planned up front. Drift found during the sweep is dispositioned by this standing rule
(inherited from phase 3, one level wider):

- **Doc/wording drift** (spec §Surface or §References out of date with shipped behaviour, a
  `docs-site/` page missing or contradicting a documented surface item, phase-vocabulary
  leakage, a stale `last_reviewed`) → fix inline in this phase.
- **Behaviour drift** (code does not do what the spec says, or a normative rule has no test) →
  new phase row in `outcome.md`; do not implement here.
- **Drift whose fix needs a product decision** → `## Blocked` entry naming the decision and
  candidate options. The audit never decides an `(Open Question)`.
- **Not drift:** an item the spec itself already flags under §Known Divergences / §Future
  Extensions / `(Open Question)` and that phase 2/3 classified `open`, `accurate` or
  `relocated`. Criterion 4's "no drift" means no *unflagged* divergence; the honestly-flagged
  open bullets are the program's declared boundary. Each report must say this explicitly and
  cite `baseline-inventory.md` rather than re-litigating those bullets.

## Tests

Gates, not unit tests (docs/audit phase):

1. `check-validations.sh` (new, in this outcome's directory) — asserts all four reports exist at
   `docs/validations/2026-09-04-<slug>-closure.md`, each carries the eight `/smelt:validate`
   report sections (Automated checks, Surface drift, Semantics drift, Invariant drift,
   Timeless-oracle drift, Freshness, Summary), and that no `❌` line survives without a
   trailing disposition marker (`— fixed this phase`, `— phase row <N>`, `— blocked`, or
   `— flagged-open: <ID>`). Red first: run it before writing any report and watch it fail on
   four missing files.
2. `check-validations.sh` green after the four reports land.
3. `check-inventory.sh` and `check-classification.sh` still green (no baseline or classification
   row may move in this phase; if a report proves one wrong, fix the row and say so in the
   summary).

## Tasks

1. Write `check-validations.sh` and confirm it fails red (four missing reports).
2. Run the shared automated-check leg **once**: `bash .claude/scripts/verify-phase.sh`. Record
   the pass/fail per gate; all four reports cite this single run (do not re-run `cargo test`
   four times).
3. For each slug in `definition_deltas`, `incremental_models`, `incremental_shapes`,
   `model_properties`, execute `/smelt:validate` steps 3–6 (Surface, Semantics, Invariant,
   Timeless-oracle, Freshness) against `docs/specs/<slug>.md`, and write the report to
   `docs/validations/2026-09-04-<slug>-closure.md`. Note in the `incremental_shapes` report that
   `docs/validations/2026-09-04-incremental_shapes.md` is the earlier *scoped* partition-grain
   validation, and that this one is the full-spec sweep that supersedes its scope.
4. Apply the doc/wording fixes for every `❌`/`⚠️` that the standing rule assigns to this phase
   (including refreshing each spec's `last_reviewed` and §References → Code/Tests/User docs when
   the freshness check flags them), and mark the report line `— fixed this phase`.
5. For anything the rule assigns elsewhere: add the phase row (behaviour drift) or the dated
   `## Blocked` entry (decision), and mark the report line accordingly. Do not implement
   behaviour changes in this phase.
6. Re-run `check-validations.sh`, `check-inventory.sh`, `check-classification.sh` — all green.
7. Write `phases/04-summary.md` carrying: per-spec drift counts and their dispositions, whether
   any new phase row or `## Blocked` entry was added, and the exact report paths phase 5's
   closure report must cite. Append a dated Decision-log entry to `outcome.md` and flip row 4
   to `done`.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be all green (re-run at the end only if a task
  touched Rust source; a docs-only phase may cite the task-2 run).
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-validations.sh`
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh`
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-classification.sh`
- `rg -nE "Phase [A-Z0-9]+" docs/specs/{definition_deltas,incremental_models,incremental_shapes,model_properties}.md`
  — every hit must sit in §Known Divergences/§Open Questions next to a `docs/plans/` link, or in
  §References.

## Commit message

`outcome(20260815-incremental-spec-closure-confirm): validate all four anchor specs and fix drift`
