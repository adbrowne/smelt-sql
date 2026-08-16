# Phase 5 plan — Overview / Design / Constraints / Limitations / Future Extensions / References pass

## Objective

Redraft the five non-Semantics prose sections of `docs/specs/incremental_models.md` (plus the one
`§Surface` polemic sentence at `:156`): delete the two anti-exclusivity polemics, strip
plan-vocabulary leaks, collapse Design/Constraints prose that merely restates the now-redrafted
§Semantics, and compress §References' narrative test essays to citations. Advances criterion 2
(polemics gone), criterion 4 (plan-vocabulary leaks gone, timeless grep clean) and the outcome
statement's length goal.

## Scope (exact ranges, at HEAD)

| Section | Lines | Now | Target |
|---|---|---|---|
| `## Overview` | 16–140 | 125 | ≤ 110 |
| `§Surface` "The declared shape" polemic sentence | 156 | 1 sentence | deleted |
| `## Design` (+ two `###` children) | 1619–1881 | 263 | ≤ 130 |
| `## Constraints & Invariants` | 1882–2019 | 138 | ≤ 95 |
| `## Limitations` | 2020–2094 | 75 | ≤ 55 |
| `## Future Extensions` | 2435–2481 | 47 | ≤ 40 |
| `## References` | 2482–2626 | 145 | ≤ 70 |
| **Total in scope** | | **793** | **≤ 500** |

Do **not** cross into: `## Semantics` (rows 2–4, done — cite it, never restate it),
`## Known Divergences / Open Questions` (row 6), the dead `IncrementalStrategy` variants /
`batched.*` / `nondeterministic_columns` / `grain: key_per_partition` fossils (row 7 — they are
*named* in Design/Constraints/Overview text; leave the fossil references standing so row 7 removes
them in one sweep), `docs-site/` (row 8).

## Spec delta

This phase *is* the spec edit; it is descriptive consolidation with no user-visible behaviour
change, so no `docs-site/` change and no diagnostic-code change is in scope. The edits:

1. **`incremental_models.md:156`** — delete the trailing sentence "Text anywhere in this corpus
   that treats … is wrong and is corrected against this section." The orthogonality claim in the
   preceding sentences stands unchanged.
2. **`incremental_models.md` §Design "The axes compose; exclusivity is the recurring error."** —
   retitle to a non-combative statement of the same decision, keep the composed-shape-is-
   first-class fact and its two cross-references, delete "the recurring error", the
   repeatedly-produced-designs catalogue, and "reviewers should treat one-or-the-other phrasing
   anywhere in the corpus as a defect".
3. **§References** — delete "for the cells this phase lifted" (plan vocabulary) and rewrite the
   contract/plan/graph-layer **Tests** bullet from multi-paragraph narrative into
   `path — one clause of what it gates` lines. Gate *names* and env knobs
   (`SMELT_CONFORMANCE_CASES`, `SMELT_CONFORMANCE_COMPOSED_CASES`, the admission-rate floors,
   `coverage_matrix_is_inhabited`) survive; the prose describing how each gate works does not —
   that is §Semantics' or the test's own job.
4. **§Design / §Constraints** — every paragraph/bullet that restates a §Semantics rule collapses
   to the rule's one-line statement plus its `§"…"` citation. §Design keeps the craft rule of one
   paragraph per decision + what was rejected + research citation; §Constraints stays an
   enumerated must-list (numbering of the two per-shape lists is preserved).
5. **§Overview / §Limitations / §Future Extensions** — trim restatement only: no bullet, boundary
   or extension is dropped.

## Tests

Red-green via `phases/05-check.sh` (new; modelled on `04-check.sh`, written before the redraft so
every check starts red on the target it asserts):

1. `structure` — the expected `##`/`###` heading list for the six in-scope sections, in order, at
   the expected levels. All existing heading strings preserved verbatim except the one §Design
   bold-lead retitle (which is not a heading).
2. `no_polemic` — `rg` finds zero of: `is wrong and is corrected`, `recurring error`,
   `reviewers should treat`, `mutually exclusive alternatives` in the spec.
3. `timeless` — `rg 'Phase [A-Z0-9]|this phase|this outcome'` clean in the spec body.
4. `claims` — every claim id in `phases/05-claims.md` marked `preserved` (fixture-driven, same
   shape as `04-check.sh`).
5. `orphan_refs` — every `§"…"` citation *in the whole file* resolves to a heading in the file (or
   is qualified by another spec's filename). Whole-file, not range-scoped — per the phase-4
   summary's finding that a heading removed in one phase is cited from sections another owns.
6. `budget` — each in-scope section is within its target above; total ≤ 500.
7. `gates_named` — the gate command strings `maintenance_conformance`, `statement_parity`,
   `execute_parity`, `walk_coverage`, `coverage_matrix_is_inhabited` still appear in §References.
8. `no_split_code_spans` — no backtick span broken across a line wrap.

## Tasks

1. Build `phases/05-claims.md`: a numbered claim inventory of the 793 in-scope lines (one row per
   normative statement, rejected alternative, research citation, gate name, and boundary).
2. Write `phases/05-check.sh` with the eight checks; confirm 2/3/6 fail red at HEAD.
3. Redraft §Overview (trim "Why cells differ" to its cost-summary; the verb/addressing detail is
   already in §"Per-cell write addressing").
4. Delete the `:156` polemic sentence.
5. Redraft §Design: intro + shared decisions, then `### Partition-grain design`,
   `### Key-grain design`. Apply spec-delta items 2 and 4.
6. Redraft §Constraints & Invariants (item 4); preserve list numbering and every named diagnostic.
7. Trim §Limitations and §Future Extensions (restatement only).
8. Rewrite §References Code/Tests bullets to citation form (item 3).
9. Run an independent adversarial-verify subagent over `05-claims.md` against the redrafted text;
   restore every outright loss and every high-value weakening; record what is left weakened.
10. Write `phases/05-summary.md` (shipped / decisions / for-the-next-planner / gates).

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/05-check.sh` → 8/8 PASS.
- Re-run `phases/02-check.sh`, `03-check.sh`, `04-check.sh` → still green (this phase edits
  cross-references into their ranges).
- Adversarial claim verification: 0 lost claims, 0 diagnostic codes dropped.
- `bash .claude/scripts/verify-phase.sh` (needs `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/`LIBRARY_PATH`
  set to `~/.local/lib/duckdb`).

## Commit message

`docs(incremental-spec): redraft the overview, design, constraints and references sections`
