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

### Phase 3 — Redraft: Semantics — **done** (2026-07-22)

- Shared machinery first, then the four shape profiles, per the target architecture.
- Commit: `docs(specs): incremental_models redraft — Semantics`

### Phase 4 — Redraft: Design, Constraints, Divergences, Extensions, References — **done** (2026-07-22)

- Design distilled; Constraints deduplicated; Known Divergences pruned to live gaps
  (deletions logged in Appendix B with the verifying gate/test for each); Future Extensions
  reviewed; References updated.
- Commit: `docs(specs): incremental_models redraft — Design through References`

### Phase 5 — Verification — **done** (2026-07-22, except `/smelt:validate` — moved to Phase 6's opening step to stay inside the session usage budget)

**Results:** 547 claims verified by six adversarial legs — 0 lost, 12 weakened (all fixed in
the two verification-fix commits), 1 retired-per-plan (the "supersedes four earlier specs"
history note), deletions/promotions all verified in place (E24/E40 repo evidence confirmed in
`config.rs`/`metadata.rs`). Notable verifier catches beyond the weakenings: a missing
Known-Divergences entry the graph layer pointed at (out-of-band-edit tripwire — added), a
stale ledger-rationale cross-reference (fixed), and confirmation that the `refresh: batched`
hard-error rule correctly lives in `models.md` as the `refresh:`-axis owner. Banned-vocabulary
lint clean; all internal §-references resolve. Verdict files: scratchpad `verify-{A,B,C,D,EF}.md`
(session-local; summaries recorded here).

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

- Run `/smelt:validate incremental_models` first and triage the drift report (deferred from
  Phase 5).
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
| The plan (derived, reported) *(holding)* | merged into §Semantics "The plan matrix" |
| Triggers *(holding)* | merged into "The plan matrix" |
| Upstream model edges *(holding)* | merged into "The graph layer" |
| The composition contract *(holding)* | merged into new §Semantics "Shape profiles" intro |
| Partition-grain / Key-grain / Interval-versioning composition *(holding)* | composition tables now open their profile's §Semantics section |
| Strategy enum (backend-internal) *(holding)* | → §Semantics partition grain "Strategy enum (backend-internal)" |
| Granularity values *(holding)* | dropped as heading; inline pointer to `timeseries.md` |
| Safety checks (per-cell admission for the partition grain's recompute corner) | → "Safety checks (per-cell admission for the recompute corner)" |
| Event-time outer-visibility (partition-grain-local) | → "Event-time outer-visibility" |
| Observing the per-source clamp (partition-grain-local surface) | → "Observing the per-source clamp" |
| Functions inside partition-grain model bodies | → "Functions inside partition-grain bodies" |
| What the composed shape uniquely enables | → "What the composed shape enables" |
| *(all other §Semantics headings)* | kept verbatim; prose tightened, content preserved |
| *(Design)* Partition-grain / Key-grain / Interval-versioning design | kept; shared-Design decision paragraphs kept with research cited by full path; "The two mechanisms stay binary per cell" folded into the locality paragraph; E57's standalone-classifier decision promoted into "Interval-versioning design" |
| *(Constraints)* all three constraint sub-section headings | kept; bullets deduplicated, numbering preserved |
| *(Known Divergences)* four sub-section headings | kept; entries rewritten gap-first from the claim inventory's E-records |
| Future Extensions | kept; "Conditional maintenance without a change feed" graduated out (see Appendix B); observer/prefix-consistency contract entry added (was inline in the old invariant section) |
| References | kept verbatim |

## Appendix B — deleted Known-Divergences entries

Filled in during Phase 4. Each row: the deleted entry's first bold phrase, and the
gate/test/code location that verifies the described work is landed.

| Deleted entry (claim id) | Disposition / verified by |
|---|---|
| "The mode value is cut and the sub-block is retired." (E24) | LANDED — `refresh: batched` hard error + `batched:` sub-block `YamlParseError` fix-its (`crates/smelt-core/src/config.rs`, `crates/smelt-core/src/metadata.rs`). Phase 5 re-verifies. The deliberate no-`smelt migrate` decision is a fix-it design already implied by the diagnostics; not restated. |
| "The pre-cut surface is removed." (E40) | LANDED — retired `refresh:` names hard-error with fix-its (`crates/smelt-core/src/config.rs`); the retired diagnostic's surviving case is covered by `PartitionGrainRequiresRefreshIncremental`. Phase 5 re-verifies. |
| "The time-partitioned keyed output's admission … are all wired." (E44) | LANDED — locality gate, settle bound, downstream pushdown/driving-source selection all normative in §Semantics; residual gaps live on in the locality-gaps and conditional-maintenance entries (E16/E45 residues kept). Phase 5 re-verifies no residue lost. |
| "Three execution paths in `crates/smelt-cli/src/main.rs`." (E33) | STALE — verified this session: `main.rs` has no legacy/optimizer/batched-only dispatch and no `PartitionGrainConfig` (CLI incremental path unified into `smelt-runtime` 2026-07-06). |
| "Diagnostic code ownership." (E37) | PROMOTED — a standing rule, not a divergence; now the ownership sentence opening §Surface "Diagnostics". |
| "Umbrella subsumption." (E57) | PROMOTED — a settled decision; now the "standalone classifier" paragraph in §Design "Interval-versioning design". |
| Future Extension "Conditional maintenance without a change feed" (F4/M1–M3) | GRADUATED — the built mechanisms are normative in §Semantics (pruning category 2, observed deltas on model edges) / `model_transforms.md` / `sources.md`; residual unbuilt pieces are Known-Divergences entries (conditional-maintenance gaps, observed-delta consumption, override-ladder reach). |
| Landed halves of the 23 MIXED entries (E1–E11, E14–E16, E18–E20, E25, E29, E36, E39, E41, E42) | Trimmed to their live residue; the full landed-evidence text is preserved in the claim inventory's E-records for Phase 5 verification. E42's residue merged into the ledger-substrate entry (was duplicated with E10). |
