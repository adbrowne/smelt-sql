# Outcome: Delta-signature front door — explain prints it, the user docs describe it

**Created:** 2026-09-04
**Status:** done
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
| 4 | Rename `guide/backbuild-synthesis.md` to `guide/migrations.md` (the `smelt migrate` verb); nav, cross-links, doc-sync gate path, retired-verb ratchet | done |
| 5 | Validate + close out: TODO bullet removed, `/smelt:validate incremental_models` clean, gates green | done |

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

- 2026-09-05 (plan, phase 4): criterion 3's literal "renamed to the `rebuild` verb" is
  **not** taken literally — `smelt rebuild` is a different shipped verb (reprocess a model
  and its upstreams over a time range under an unchanged definition), and
  `guide/incremental-models.md` already contrasts the two. Every worked example on the page
  drives `smelt migrate`, and `reference/cli.md` already calls it "the migration guide", so
  the rename target is `docs-site/docs/guide/migrations.md` / `# Migrations`. Naming it
  `rebuild.md` would manufacture the exact collision the 2026-08-29 verb rename ended. The
  outcome's prose intent ("the guide page that still carries the retired `backbuild` verb is
  renamed") and criterion 3's checkable half are both satisfied.
- 2026-09-05 (plan, phase 4): criterion 3's "`rg backbuild docs-site/docs` finds only a
  historical note, if any" is enforced as a standing ratchet in `docs_front_door.rs` with two
  documented exemptions — `__backbuild_diff`/`__backbuild_branch` are alias names *emitted by*
  `smelt-logical/src/backbuild/emit.rs` into real SQL and pinned by the conformance suite, and
  backticked code spans naming real symbols (`derive_backbuild_options`,
  `tests/backbuild_docs.rs`). Renaming the `smelt_logical::backbuild` module or its emitted
  aliases is a code rename this outcome does not call for; the 2026-08-29 decision explicitly
  kept the mechanism name.
- 2026-09-05 (plan, phase 4): no reshape of row 5 — the phase 3 summary reported rows 4 and 5
  unaffected, and this phase's audit found nothing that changes the close-out row. One task is
  *added inside* row 4 rather than deferred: a `docs_site_relative_links_resolve` gate, because
  a rename with four inbound cross-links and no link check would put criteria 2-4 one typo away
  from silent rot.
- 2026-09-05 (implement, phase 4): shipped the rename (`git mv`), the two new
  `docs_front_door.rs` gates (5/5 green), the retitled/reworded page, marker-id rename
  (`backbuild-example` → `migrate-example`, 42 markers), `backbuild_docs.rs`'s `GUIDE_PATH` +
  marker constants + doc comment (7/7 green, unchanged assertion count, no regen needed), the
  `mkdocs.yml` nav entry, all four inbound cross-links, and the `definition_deltas.md` §References
  path. `verify-phase.sh` ALL GREEN; see `phases/04-summary.md`.

- 2026-09-05 (plan, phase 5): criterion 5's `/smelt:validate incremental_models` is scoped to
  §Surface "CLI" (spec lines 417-541) plus the §References **User docs** block. A full-spec
  validate over a 2,240-line normative spec is a multi-hour sweep whose semantics/invariants
  legs this outcome does not ask for, and `docs/TODO.md` already carries a separate bullet for
  the three-spec drift baseline — which stays, as out-of-scope work. The report names what it
  did not validate.
- 2026-09-05 (plan, phase 5): the validation lands as a committed artifact
  (`docs/validations/2026-09-05-incremental_models-cli-surface.md`, following the
  `2026-09-04-definition_deltas-closure.md` precedent) so the close-out judgement has evidence
  to read rather than a transcript to trust.
- 2026-09-05 (plan, phase 5): no reshape - phase 5 is the last row, the phase 4 summary
  deferred nothing, and this phase's audit found the divergence bullet already narrowed
  (criterion 1) and the four-corners text already absent (criterion 2). One task is added
  *inside* row 5 rather than deferred: a `spec_user_docs_block_lists_existing_pages` gate,
  because phase 4's rename showed the spec's References block can point at a deleted docs-site
  path with no gate noticing.

- 2026-09-05 (implement, phase 5): found and fixed a real spec bug —
  `incremental_models.md` claimed `smelt rebuild --event-time-start/--event-time-end
  [selectors]`; the actual CLI takes `<selector> --start --end` (docs-site was already
  correct). The prior `2026-09-04-definition_deltas-closure.md` validation had asserted the
  wrong flags as ✅ by trusting the spec's own text instead of `main.rs`. Also removed a
  stale "not yet implemented" doc comment on `BakeoffArgs::pin` (it's shipped). Shipped
  `docs_front_door::spec_user_docs_block_lists_existing_pages`, bumped `last_reviewed`, and
  committed `docs/validations/2026-09-05-incremental_models-cli-surface.md`.
  `verify-phase.sh` ALL GREEN; see `phases/05-summary.md`. All five outcome success criteria
  are met — outcome closed as done.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
