# Outcome: Delta-signature front door — explain prints it, the user docs describe it

**Created:** 2026-09-04
**Status:** active
**Source:** `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md` row IM-02; `docs/TODO.md` §"docs-site sync"; `docs/research/20260904-incremental-state-review.md` §"What went wrong" (docs-site lag) and §"Recommended next sequence" item 6
**Spec anchors:** `docs/specs/incremental_models.md` §Surface "CLI" and the delta-signature sections, `docs/specs/definition_deltas.md`, `docs/specs/incremental_shapes.md`

## The outcome

`smelt explain <model>` opens with the model's delta-signature headline exactly as
`incremental_models.md` §Surface "CLI" specifies, in text and `--json`. The docs-site guide for
incremental models is written around the same front door the specs adopted on 2026-08-12: a
reader learns what a delta signature is, sees the headline `smelt explain` prints for the
tutorial models, and is pointed from there to shapes, contracts, `smelt migrate` and
`smelt rebuild`. The four-corners framing is gone from the user docs, and the guide page that
still carries the retired `backbuild` verb is renamed. Doc-sync gates keep the printed output and
the tutorial pages in step.

## Success criteria (checkable)

1. `smelt explain` prints the delta-signature headline first, matching the spec's stated form,
   for every example workspace model; `--json` carries the same fields; `cli_docs_coverage`
   and `explain_*` tests cover it. `incremental_models.md` §Known Divergences bullet "does not
   yet print the delta-signature headline" is deleted.
2. `docs-site/docs/guide/incremental-models.md` introduces delta signatures before any shape or
   contract detail; the word "signature" appears in its first section; no "four corners" text
   remains anywhere under `docs-site/docs/`.
3. `docs-site/docs/guide/backbuild-synthesis.md` is renamed to the `rebuild` verb, with the nav
   entry and every cross-link updated; `rg backbuild docs-site/docs` finds only a historical
   note, if any.
4. Every `smelt explain` excerpt in the docs-site is generated from the tutorial doc-sync
   pipeline (`tutorial_pages/` templates), so the new headline appears in every excerpt after
   one regeneration; the tutorial freshness gate is green.
5. `docs/TODO.md` §"docs-site sync" bullet is removed; `/smelt:validate incremental_models`
   reports no drift for the CLI surface; `verify-phase.sh` green.

## Out of scope

- The scheduler consuming delta signatures end to end (IM-01; `scheduler-delta-signatures`
  needs a human-reviewed first plan).
- The per-column guarantee ledger and forward-reach output in `smelt explain` (IS-19,
  "proofs as product").
- Any change to what a delta signature is; the spec's definition is taken as given.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | `smelt explain` delta-signature headline: text + `--json`, tests, divergence bullet deleted | done |
| 2 | Regenerate tutorial pages; re-derive the two hand-pasted explain excerpts (`reference/cli.md`, `guide/incremental-models.md`) from real output; standing headline gate; freshness gate green | done |
| 3 | Rewrite `guide/incremental-models.md` around delta signatures; purge four-corners text across docs-site | done |
| 4 | Rename `guide/backbuild-synthesis.md` to the rebuild verb; nav + cross-links | pending |
| 5 | Validate + close out: TODO bullet removed, `/smelt:validate incremental_models` clean, gates green | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-09-05 (plan, phase 1): criterion 1's "every example workspace model" is read as
  every example model that *has* a maintenance-plan report — the spec gives a delta
  signature to declared sources and maintained models only, so the no-plan notice path
  keeps its existing one-line output rather than fabricating a signature for a
  full-refresh model.
- 2026-09-05 (plan, phase 1): the Known Divergences bullet is **narrowed**, not deleted —
  its headline clause goes, but its per-column guarantee-ledger and derived-run-shape
  clauses describe IS-19 work this outcome lists under "Out of scope".
- 2026-09-05 (plan, phase 1): no reshape of the phase table — phase 1 is the first row and
  no prior phase summary exists.
- 2026-09-05 (plan, phase 2): criterion 4 is read as "no stale hand-pasted explain output
  anywhere", not "the web-analytics tutorial pipeline is extended to reference/guide pages" —
  `generate_tutorial.py` is scoped to `docs-site/docs/examples/web-analytics/` and widening it
  is a pipeline rewrite this outcome does not call for. The two non-pipeline excerpts
  (`reference/cli.md` `daily_events`, `guide/incremental-models.md` `daily_events_enriched`)
  are instead re-derived from real `smelt explain` runs against `examples/timeseries` and
  pinned by a new standing gate.
- 2026-09-05 (plan, phase 2): phase 2's row is widened to name that excerpt work (the audit
  found exactly three `Maintenance plan:` blocks in the docs-site, two of them stale); no rows
  added, split, or removed — the widened work serves criterion 4 and stays inside this phase.
- 2026-09-05 (implement, phase 1): shipped `smelt_db::model_output_delta_for`,
  `DeltaSignatureHeadline`, and both report/JSON builders wired to it; 7 new tests; spec/doc
  edits landed. A `None` own-shape (no derivable output-delta at all) renders as a degraded
  `general` verdict rather than a third render path. Regenerated the golden `--show-sql` fixture
  and one tutorial-doc excerpt (`deduplication.md`) to keep `verify-phase.sh` green — both
  2-line diffs, the new headline only. `verify-phase.sh` ALL GREEN; see
  `phases/01-summary.md`.
- 2026-09-05 (implement, phase 2): shipped `crates/smelt-cli/tests/explain_docs_freshness.rs`
  (3 tests: headline-first gate across all docs-site excerpts, byte-pin for `reference/cli.md`,
  un-elided-prefix pin for `guide/incremental-models.md`); regenerated both hand-pasted excerpts
  from real `smelt explain` output; tutorial pages already fresh (no diff). `verify-phase.sh` ALL
  GREEN; see `phases/02-summary.md`.
- 2026-09-05 (plan, phase 3): criterion 2's "no four corners text remains anywhere under
  `docs-site/docs/`" is already literally true (`rg -in "four.corners" docs-site/docs/` is
  empty today — the four-corners grid lives only in `docs/specs/models.md`). The phase
  therefore spends its effort on the signature-first rewrite and converts the four-corners
  clause into a standing ratchet test rather than a removal task, so the criterion cannot
  quietly regress later.
- 2026-09-05 (plan, phase 3): the guide's current front door is the DELETE+INSERT mechanics
  paragraph, which states a *fixed* strategy — contradicting the spec's derived-per-cell
  maintenance plan. Demoting it under §"Running incremental models" is folded into this phase
  rather than deferred: leaving a fixed-strategy claim above the new signature front door
  would defeat criterion 2 in substance while passing it in letter.
- 2026-09-05 (plan, phase 3): no reshape of rows 4 and 5 — the phase 2 summary reported both
  unaffected, and this phase's audit found nothing that changes them.
- 2026-09-05 (implement, phase 3): shipped the signature-first `## What a model emits` section
  (fronted with a real `smelt explain user_daily_spend` headline, chosen because it emits
  `keyed upsert` rather than degrading to `general`) and demoted DELETE+INSERT mechanics to
  `### What a partition-shaped run does`; new gate `crates/smelt-cli/tests/docs_front_door.rs`
  (3 tests: first-section content, headline byte-pin, four-corners ratchet). All 11 inbound
  anchors preserved. `verify-phase.sh` ALL GREEN; see `phases/03-summary.md`.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
