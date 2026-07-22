# CLAUDE.md — writing and editing specs

Guidance for working in `docs/specs/`. `SPEC_TEMPLATE.md` owns the file *structure*; the root
`CLAUDE.md` owns the process rules (spec-first, timeless-oracle). This file owns the *craft*:
how to write a spec both humans and Claude can actually digest. A spec is the oracle the team
aligns on — if a reader has to re-read a section twice, the spec has failed at its one job.

## The pyramid rule

State the conclusion first, then the support. This applies at every zoom level:

- **File level.** The feature's central guarantee comes first; machinery that serves it comes
  after. A large spec (roughly 800+ lines) should open with a non-normative `## Overview`
  primer (sanctioned by the template): the core invariant in plain English then formally, the
  declared surface in outline, the derived machinery in one paragraph each, a reading guide.
- **Section level.** Open with what the section owns and its one-sentence rule; details,
  tables, and edge cases follow.
- **Sentence level.** One qualification per sentence. A load-bearing qualification gets its own
  sentence, not another em-dash clause. If a sentence needs three parentheticals, it is three
  sentences.

## Concept ordering

Every term is defined before first use — the test is a reader starting at line 1 who never has
to jump ahead. Forward references are allowed only as explicit pointers: "(detailed in §X)".
When the template's section order fights concept order (Surface uses a Semantics concept), fix
it by introducing the concept in the Overview, not by making Surface carry semantics.

## Examples are load-bearing

- Give the spec one small **running example** (a mini warehouse, a handful of models) and use
  it everywhere; per-section throwaway examples cost more context than they buy.
- The highest-leverage artifact for derived behaviour is a **rendered tool output** (e.g. a
  `smelt explain` plan for one concrete model): one illustration can replace pages of abstract
  prose about what "is derived per cell" means. Mark it illustrative, not normative.
- Every declared surface (frontmatter key, CLI flag) gets at least one worked snippet.

## One home per statement

- Each doctrine is stated normatively **exactly once**; everywhere else it is referenced by
  its section name. If you find yourself restating a rule "for emphasis", link instead.
- Never re-specify what a sibling spec owns. Name the owner and cite it
  (`sources.md §"…"`). If ownership is unclear, that's a spec bug to fix, not to paper over.
- Diagnostics: one unified table in §Surface (code → when it fires → owning section). Codes
  mentioned in prose must appear in that table.

## The timeless oracle, enforced

The root `CLAUDE.md` states the rule; these are the failure modes that actually creep in:

- **Arguing with past drafts.** A section written to correct an earlier version ("X is a
  category error", "text that says Y must be corrected") becomes one calm normative sentence.
  The rule survives; the polemic goes.
- **Historical names and states.** No "(Historical name: …)", "pre-X", "the old surface",
  "supersedes …". Git remembers; the spec doesn't.
- **Research shorthand.** Cite research by full path (`docs/research/2026….md §n`). Never
  bare part-numbers ("01 §5"), internal decision labels ("P3", "D6") without the file that
  defines them, or process vocabulary ("ratified").
- Grep before committing: `rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]' docs/specs/<file>.md`

## Known Divergences is a gap list, not a changelog

- Entries are **gap-first**: lead with what is missing or undecided, not with what landed.
  "X has landed; remaining gap Y" becomes "**Y.** …".
- A fully-landed entry is not a divergence — delete it. History lives in git and
  §References → Plans.
- A settled decision parked in Known Divergences gets **promoted** into §Design or the body.
- Every gap names its tracking plan by full path; every open question states the current
  best-known answer.

## §Design discipline

One paragraph per decision: the decision, the rejected alternative(s), the reason, the
full-path research citation. Rejected alternatives are first-class content — they prevent
relitigating settled questions — but keep each to a clause, not an essay.

## Heading names are API

Section names are referenced by name (`§"…"`) from code comments, sibling specs, and plans —
hundreds of references for a large spec. Treat a heading rename like a symbol rename: do it
when the name is genuinely poor, and sweep the corpus (code comments and sibling specs;
`docs/plans/` and `docs/research/` are historical records and stay untouched). Check the blast
radius first: `rg '§"<name>"' --glob '!docs/plans' --glob '!docs/research'`.

## Large redrafts: the claim-inventory method

A structural rewrite of a spec must be **semantics-preserving**, and "I was careful" is not
verification. The method that works (worked example:
`docs/plans/20260722-incremental-models-spec-redraft.md`):

1. **Inventory first.** Extract every normative claim (must/refuse/diagnostic/default/
   definition/ownership/carve-out) into a numbered list with source line ranges, before
   touching the text. Subagents parallelise this well.
2. **Classify Known Divergences** (landed / live / mixed) before pruning; verify "landed"
   against the repo, not the entry's own say-so — entries go stale.
3. **Rewrite** keeping an old→new heading map as you go.
4. **Adversarially verify**: independent checkers grade every inventoried claim against the
   new text (preserved / weakened / lost / strengthened) and you fix everything that isn't
   `preserved`. Expect real catches — dropped diagnostic codes, dropped rejection rationale,
   dangling "(§Known Divergences)" pointers.
5. Lint: banned vocabulary, and that every internal `§"…"` reference resolves to a heading.

## Calibration

- Prefer prose in complete sentences; tables only for genuinely enumerable facts (families,
  diagnostic codes, admission matrices) with the explanation in surrounding prose.
- Shorter is only better when it comes from *dropping restatements and history*, never from
  compressing rules into fragments. A rule's exact strength ("hard error" vs "warning",
  "refused" vs "discouraged", "never" vs "not yet") is content — preserve the words that
  carry it.
