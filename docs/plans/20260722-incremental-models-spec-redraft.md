# Incremental-models spec redraft

**Spec:** `docs/specs/incremental_models.md`
**Spec diff:** none — this plan is a *semantics-preserving redraft* of the spec itself. The
normative content must survive unchanged; only structure, prose, and examples change.
**Docs:** spec + `docs/specs/SPEC_TEMPLATE.md` only. No code changes. `docs-site/` untouched
(the user docs already describe the same behaviour; the redraft does not change behaviour).

## Why

`incremental_models.md` has grown to ~2,600 lines through successive redrafts and reads poorly:

1. **Inverted pyramid.** The parent concept — the equivalence invariant — is defined at line
   ~543, after 520 lines of Surface that forward-reference it. The opening blockquote uses
   ~20 terms before any is defined.
2. **Redraft scar tissue.** Sections argue with earlier drafts ("category error", "must be
   corrected against this section", "(Historical name: 'batched')", "pre-consolidation") —
   timeless-oracle violations.
3. **Research shorthand leaked in**: "(01 §5)", "ratified P3/P4", "grain-demotion",
   "pre-cut", "keyed-collapse" — indecipherable without the research corpus.
4. **Known Divergences is ~700 lines (27%)**, much of it a changelog of *landed* work.
5. **Doctrines restated 3–5× at full strength** (validator-not-chooser, only-proofs-prune,
   per-cell addressing, orthogonality).
6. **No worked examples** beyond frontmatter snippets.

The spec is the oracle both developers and Claude align on; this redraft is a long-term
investment in its digestibility.

## Decisions (agreed 2026-07-22)

- **One file, pyramid order, good names.** Single spec rebuilt in strict concept order.
  Headings are renamed where the current name is poor; the cross-corpus reference sweep is
  **deferred** to a follow-up after the spec settles (old→new mapping kept in this plan).
- **Landed Known-Divergences entries are deleted outright** (history lives in git and
  §References → Plans). Each deletion is verified against the CI gates/tests first.
- **`SPEC_TEMPLATE.md` gains an optional non-normative `## Overview` section** allowed
  before `## Surface`, giving the mental-model primer a sanctioned home.
- The redraft is written in-session by the lead model (judgment-heavy prose); subagents are
  used for the verification pass, not the writing.

## Target architecture

```
# Incremental Models
> What this is — 5-line scope blockquote

## Overview                      (new; non-normative primer, ~2 pages)
  1. The one guarantee: the equivalence invariant, plain then formal
  2. What you declare: clock + identity (+ versioning: interval); all else derived
  3. The four corners: 2×2 table, 3-line example each
  4. How smelt maintains it: the plan (cells × triggers × changed-input) in one paragraph
  5. The running example, introduced
  6. Reading guide

## Surface                       (pure declaration reference)
  declared facts + check-only grain · maintenance: frontmatter · per-shape declaration
  reference (partition / key / composed / interval), one example each · CLI · one unified
  diagnostics table

## Semantics                     (shared machinery first, then shape profiles)
  equivalence invariant (full statement) · the plan: matrix, per-cell admission, per-cell
  write addressing, interchangeability · windows/clamps/horizon + only-proofs-prune (once) ·
  validator-not-chooser · statement emission · ledger · graph layer · shape profiles
  (partition → key → locality/composed → interval), each = composition table + local
  machinery only · definition-change trigger · interactions

## Design                        (one paragraph per decision; research cited by full path)
## Constraints & Invariants      (existing checklist, deduplicated)
## Known Divergences / Open Questions   (live gaps only; target ~150 lines from ~700)
## Future Extensions             (kept, staleness-reviewed)
## References                    (kept, updated)
```

**Running example** used consistently throughout: clocked `orders` fact (with lateness),
mutable `customers` dimension, and four models — `daily_revenue` (partition grain),
`order_lifecycle` (key grain), `event_dedupe` (composed shape, recurrence-bounded locality),
`customer_history` (SCD2). Centerpiece: a rendered `smelt explain` plan for a composed model
showing the creation cell deriving a region rewrite while the dimension-change cell derives a
keyed merge.

## Writing rules (review checklist for every redrafted section)

- [ ] Every term defined before first use; forward references only as explicit "(detailed in §X)".
- [ ] Each doctrine stated normatively exactly once; elsewhere referenced by name.
- [ ] Banned vocabulary (grep-enforced in Phase 5): `ratified`, `pre-cut`, `grain-demotion`,
      `Historical name`, `pre-consolidation`, `category error`, bare research citations
      (`01 §`, `03-design-forks`, `08-code-placement`, `10-dependency-propagation`,
      `» §\d+` shorthand), plus the timeless-oracle rule's existing bans (`Phase [A-Z0-9]`).
- [ ] Guard sections become one calm normative rule each; no polemics addressed at past drafts.
- [ ] Sentences carry one qualification; load-bearing qualifications get their own sentence.
- [ ] Normative force preserved: every must/refuse/diagnostic/carve-out from the claim
      inventory survives with meaning intact.

## Phases

Progress key: `pending` / `in-progress` / `done` / `blocked`.

### Phase 0 — Plan + template amendment — **done** (2026-07-22)

- Commit this plan.
- `SPEC_TEMPLATE.md`: add optional `## Overview` (non-normative, allowed between the scope
  blockquote and `## Surface`); update the frontmatter comment's blockquote-placement rule
  to match.
- Commit: `docs(plans): incremental-models spec redraft plan; SPEC_TEMPLATE optional Overview`

### Phase 1 — Claim inventory — **done** (2026-07-22; 547 entries: A117 surface, B120 shared semantics, C127 shape profiles, D114 design+constraints, E57 divergence classifications — 3 LANDED / 23 MIXED / 31 LIVE, F12 future+references)

- Extract every normative claim (must / must-not / refuse / diagnostic / invariant /
  carve-out / default) from the current spec into
  `docs/plans/20260722-incremental-models-spec-redraft-claims.md`, numbered, each with its
  source line range. Subagent-parallel by section; spot-checked.
- This file is the verification oracle for Phase 5 and is deleted in the follow-up sweep PR
  once verification passes (it is scaffolding, not documentation).
- Commit: `docs(plans): claim inventory for incremental-models spec redraft`

### Phase 2 — Redraft: Overview + Surface — **done** (2026-07-22)

- Write the scope blockquote (≤6 lines), `## Overview`, and `## Surface` per the target
  architecture, introducing the running example.
- Maintain the heading map (Appendix A) for every renamed/moved/merged section.
- Commit: `docs(specs): incremental_models redraft — Overview + Surface`

### Phase 3 — Redraft: Semantics — **pending**

- Shared machinery first, then the four shape profiles, per the target architecture.
- Commit: `docs(specs): incremental_models redraft — Semantics`

### Phase 4 — Redraft: Design, Constraints, Divergences, Extensions, References — **pending**

- Design distilled; Constraints deduplicated; Known Divergences pruned to live gaps
  (deletions logged in Appendix B with the verifying gate/test for each); Future Extensions
  reviewed; References updated.
- Commit: `docs(specs): incremental_models redraft — Design through References`

### Phase 5 — Verification — **pending**

- Adversarial subagent pass: every inventoried claim checked against the new text
  (verdict per claim: preserved / weakened / lost / strengthened); anything not `preserved`
  is fixed or explicitly justified here.
- Terminology lint (banned-vocabulary grep) passes.
- Deleted-divergence verification: each Appendix B entry confirmed landed via its gate/test.
- `/smelt:validate incremental_models` drift report run; findings triaged (drift caused by
  the redraft is fixed; pre-existing drift is recorded, not silently absorbed).
- Heading map (Appendix A) complete: every old heading accounted for.
- Commit: `docs(specs): incremental_models redraft — verification fixes`

### Phase 6 — Reference sweep — **pending** (follow-up PR, after the spec settles)

- Update §-name references in **code comments and sibling specs** to the new headings using
  Appendix A. `docs/plans/` and `docs/research/` are historical records and stay untouched
  (standing convention).
- Delete the claim-inventory scaffolding file.
- Commit: `docs: sweep incremental_models §-references to redrafted headings`

## Appendix A — heading map (old → new)

Filled in during Phases 2–4. Every heading of the pre-redraft spec must appear exactly once.

| Old heading | Disposition (new heading / merged into / deleted) |
|---|---|
| (header status paragraph, no heading) | content → §Known Divergences (already duplicated there; verified in Phase 5 via claim A8) |
| The declared shape axis | → §Surface "The declared shape" |
| Grain is a derived label (+ optional check-only assertion) | merged into "The declared shape" |
| The two axes are orthogonal — "partitioned or keyed" is a category error | merged into "The declared shape" (one calm normative rule) |
| The composition contract | → §Semantics (holding area; final home Phase 3: intro of the shape-profile sections) |
| The plan (derived, reported) | → §Semantics (holding; Phase 3: plan machinery) |
| Triggers | → §Semantics (holding; Phase 3: plan machinery) |
| Upstream model edges | → §Semantics (holding; Phase 3: graph layer) |
| Frontmatter | → §Surface "Maintenance overrides (`maintenance:`)" |
| Partition-grain declaration (`grain: partition`) | kept (rules folded in) |
| Partition-grain composition | → §Semantics partition profile (holding; Phase 3) |
| Partition-grain frontmatter (in `.sql` files) | merged into "Partition-grain declaration" |
| Partition-grain `smelt.yml` overrides | merged into "Partition-grain declaration" |
| Granularity values | dropped as heading; pointer to `timeseries.md` retained in declaration rules (holding until Phase 3) |
| Strategy enum (backend-internal) | → §Semantics partition profile (holding; Phase 3 — backend-internal, not surface) |
| Key-grain declaration (`grain: key`) | kept (rules folded in) |
| Key-grain composition | → §Semantics key profile (holding; Phase 3) |
| Key-grain frontmatter (in `.sql` files) | merged into "Key-grain declaration" |
| Key-grain `smelt.yml` overrides | merged into "Key-grain declaration" |
| The column-family catalogue | kept |
| Interval-versioned declaration (`versioning: interval`) | kept (rules folded in) |
| Interval-versioning composition | → §Semantics interval profile (holding; Phase 3) |
| Interval-versioning frontmatter (in `.sql` files) | merged into "Interval-versioned declaration" |
| Interval-versioned output shape | merged into "Interval-versioned declaration" (validity-columns rule) |
| CLI | kept; run-flag subsections merged in |
| Partition-grain run flags | merged into "CLI" → "Run flags" |
| Key-grain run flags | merged into "CLI" → "Run flags" |
| Diagnostics | kept — one unified table (shared / partition / key groups) |
| The `Maintenance*` family | merged into "Diagnostics" |
| Key-grain diagnostic codes | merged into "Diagnostics" |
| *(Semantics onward: filled in Phase 3/4)* | |

Planned renames already referenced by the new text (final in Phase 3): "What the composed
shape uniquely enables" → "What the composed shape enables".

## Appendix B — deleted Known-Divergences entries

Filled in during Phase 4. Each row: the deleted entry's first bold phrase, and the
gate/test/code location that verifies the described work is landed.

| Deleted entry | Verified landed by |
|---|---|
| *(filled during pruning)* | |
