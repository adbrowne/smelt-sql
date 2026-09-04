# Outcome: Delta-signature front door — explain prints it, the user docs describe it

**Created:** 2026-09-04
**Status:** queued
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
| 1 | `smelt explain` delta-signature headline: text + `--json`, tests, divergence bullet deleted | pending |
| 2 | Regenerate tutorial pages; confirm every explain excerpt carries the headline; freshness gate green | pending |
| 3 | Rewrite `guide/incremental-models.md` around delta signatures; purge four-corners text across docs-site | pending |
| 4 | Rename `guide/backbuild-synthesis.md` to the rebuild verb; nav + cross-links | pending |
| 5 | Validate + close out: TODO bullet removed, `/smelt:validate incremental_models` clean, gates green | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
