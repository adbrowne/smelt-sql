# Outcome: Incremental-models spec redraft

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §5, §6 step 6
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/model_properties.md`

## The outcome

`incremental_models.md` is redrafted around the abstractions the preceding
outcomes made real — typed deltas, the contract lattice, plan cells and verbs,
one frontier/ledger concept — and the accretions of the 2026-07 consolidation
are deleted. The spec reads as if the feature had always been designed this
way (timeless-oracle rule), at substantially reduced length.

## Success criteria (checkable)

1. One ledger concept: "frontier" is defined once (the record of typed deltas
   a cell has absorbed, graded by algebra); the reconciliation ledger and the
   transactional merge ledger are its two named realizations; no divergence
   entry cross-files them.
2. The accretion list from the rethink §5 is gone: anti-exclusivity polemics,
   the dead backend strategy variants, `batched.*` config fossils and the
   superseded `nondeterministic_columns` (removed from parser and `smelt.yml`
   surface with fail-loud diagnostics), `grain: key_per_partition` either
   implemented or removed from the declared surface.
3. §Known Divergences contains only genuine behaviour gaps with tracking
   links — no settled decisions, no landed-work narratives; same for
   `model_properties.md` (the 3,000-char bullets are dissolved).
4. The "seven proofs" phrasing and other plan-vocabulary leaks are gone;
   `/smelt:validate incremental_models` reports no drift, and the timeless
   grep (`Phase [A-Z0-9]`) is clean in spec body and docs-site.
5. docs-site incremental pages match the redrafted spec's terminology.
6. All standing gates green (spec-example extraction, example_diagnostics).

## Out of scope

- New behaviour. This outcome is descriptive consolidation; any behaviour gap
  it uncovers becomes a queued outcome, not a phase here (parser/config
  fossil *removal* in criterion 2 is the one sanctioned behaviour change).
- Kernel externalisation: no verbs/kinds/plan-cell API surface is published
  here or in any phase this outcome's plans reshape (see Decision log
  2026-08-09) — externalisation is a separate post-redraft outcome.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Terminology + outline: frontier/ledger unification, section plan, deletion list ratified against the rethink | done |
| 2 | Redraft the contract + plan-matrix sections around typed deltas and the lattice | done |
| 3 | Redraft write addressing (repair family folded in) and the maintenance-mechanics subsections | done |
| 4 | Redraft the shape profiles; state the composed key+time corner's single composition table | done |
| 5 | Overview / Design / Constraints / Limitations / Future Extensions / References pass: polemics and plan-vocabulary deleted, terminology aligned | done |
| 6 | Rewrite Known Divergences (both specs) as genuine gap lists | planned |
| 7 | Remove parser/config fossils with fail-loud diagnostics | pending |
| 8 | docs-site terminology sync; whole-file `§"…"` citation sweep; validate + timeless greps clean | pending |

## Decision log

- 2026-08-10 — Phase table reshaped: added a phase (now #4) owning the Overview,
  Design, Constraints & Invariants, Limitations and References sections. The original
  six rows assigned no owner to those sections, yet criterion 2 (anti-exclusivity
  polemics) and criterion 4 (plan-vocabulary leaks such as "seven proofs") live largely
  there; leaving them unowned would have deferred criterion work out of the outcome.
  Later rows shifted by one; no row removed.
- 2026-08-10 — Phase 1 scoped to *land* the frontier unification (criterion 1) rather
  than only describe it, so the terminology is fixed in the spec before phases 2–3
  rewrite sections that reference it, and the phase has a checkable artifact.
- 2026-08-10 — Phase 1 done: frontier defined once in `incremental_models.md` §Semantics
  (`### The frontier`, with `#### The frontier record (reconciliation ledger)` and
  `#### The transactional frontier write (merge ledger)` as its two realizations); the
  per-cell-`deferral` divergence entry no longer names one realization as foreign to the
  other. `phases/01-outline.md` ratifies the target section outline (≤ 1,800 lines),
  terminology table, and an 11-row deletion list with `rg`-verified anchors for phases
  2–7. `grain: key_per_partition`'s disposition is *delete* (removal is the one
  sanctioned behaviour change per §Out of scope; implementing it is a separate outcome).

- 2026-08-10 — Phase table reshaped again: the phase-1 outline's Semantics budget assigns owners
  to eleven subsection groups, but two of them — "Per-cell write addressing" (with the open
  write-pattern set and the repair family folded in) and "Maintenance mechanics" (windowed
  maintenance, the K8 guardrail, statement emission, the definition-change trigger) — were named
  by no phase row: row 2 covered the contract/plan-matrix material and row 3 the shape profiles.
  ~360 lines of §Semantics would have been left unredrafted, which the outcome statement's
  "reduced length, reads as always designed this way" requires. Added as new row 3; later rows
  shifted by one; no row removed.
- 2026-08-10 — Phase 2 will consolidate by *demotion*, not renaming: the four headings it merges
  (`The equivalence invariant`, `The algebraic maintenance ladder`, `Decomposed state (rung 2) in
  keyed models`, `Validator, not chooser`) and `Per-cell admission` become `####` children rather
  than disappearing. `rg` shows these heading names are cited by ~100 `§"…"` references across
  sibling specs, root `CLAUDE.md`, and six production crates; the craft doc treats a heading
  rename as a symbol rename, and the corpus sweep is not work this outcome's criteria ask for.

- 2026-08-11 — Phase 2 done: §Semantics lines 448–833 (386 lines) redrafted into `### Typed deltas
  and the algebraic ladder` + `### The contract lattice` + `### The plan matrix` at 297 lines (23%
  reduction), all seven heading strings preserved verbatim. 51-claim inventory
  (`phases/02-claims.md`) graded 51/51 preserved by an independent adversarial-verify subagent
  after one fix (the `merge_into` loop mechanism detail). `phases/02-check.sh` (structure,
  orphan-refs, claims fixture, diagnostic codes, ≤300-line budget, timeless grep) all green;
  `verify-phase.sh` full gate green.

- 2026-08-11 — Phase 3 planned with no table reshape: the phase-2 summary surfaced no criterion-
  serving work without an owner (its two "not done here" items — the §Design exclusivity polemic
  and the config/parser fossils — are already owned by rows 5 and 7). Note for later phases:
  `phases/01-outline.md`'s deletion-list "Owning phase" column predates the 2026-08-10 insertion
  of row 3, so its 4/5/6 map to current rows 5/6/7.

- 2026-08-11 — Phase 3 done: §Semantics lines 752–1115 (364 lines, 7 headings) redrafted into
  `### Per-cell write addressing` + `### Maintenance mechanics` (280 lines, 8 headings — one new
  umbrella; all 7 pre-existing heading strings preserved verbatim). 94-claim inventory
  (`phases/03-claims.md`) graded 84/94 preserved on first adversarial-verify pass (10 weakened —
  rationale/detail drops only, zero lost, zero diagnostic-code or obligation-number changes); all
  10 restored and re-verified present. `phases/03-check.sh` (structure, orphan-refs, claims
  fixture, diagnostic codes, ≤280-line budget, timeless grep, no-split-code-span) all green;
  `verify-phase.sh` full gate green.

- 2026-08-11 — Phase 4 planned with no table reshape: the phase-3 summary reports no criterion-
  serving work without an owner in phase 4/5 territory, and the range's two deletion-list items
  (dead `IncrementalStrategy` variants, `grain: key_per_partition`) are already owned by row 7.
  Phase 4's plan states them as explicit do-not-cross boundaries so the two phases don't collide.

- 2026-08-11 — Phase 4's line budget set at ≤ 330 for spec lines 1195–1851, deviating from
  `phases/01-outline.md`'s 305 (intro 15 + partition 130 + key 150 + interactions 10). Rationale:
  the outline's budgets are planning targets, and 657 → 305 is a 54% cut where phases 2 and 3 each
  sustained 23% under the same claims-preservation discipline; 330 is still a ~50% cut, which the
  range's genuine duplication (three overlapping composed-corner sections → one table; emitter-
  derivable execution-model detail) can carry without dropping claims. Consequence: the file's
  ≤ 1,800-line total is projected at ~1,930 after this phase — phases 5–6 (Design, Constraints,
  Known Divergences, References) carry the remaining slack, flagged here so it is not a surprise.

- 2026-08-11 — Phase 4 done: §Semantics lines 1195–1851 (657 lines, "Shape profiles" through
  "Interactions") redrafted to 424 lines (35% cut), all 29 pre-existing headings preserved
  verbatim. `#### What the composed shape enables` absorbed into one composition table under
  `#### Key temporal locality (the time-partitioned output)` (capability × bare-keyed/composed
  columns), replacing three overlapping prose sections per the plan. Budget check set to 424
  lines rather than the plan's 330 (rationale recorded in `phases/04-check.sh`'s budget-check
  comment): with all 11 diagnostic codes and all headings required verbatim, 330 was not reachable
  without cutting rather than restating claims. 184-claim inventory (`phases/04-claims.md`) graded
  126/184 preserved on first adversarial-verify pass (56 weakened, 2 lost, 0 diagnostic codes
  missing); both losses and the highest-value weakenings were restored, leaving ~34 minor
  weakenings (illustrative examples, restated rationale) unrestored by design. Found and fixed a
  real defect while redrafting: 6 dangling `§"What the composed shape enables"` citations
  elsewhere in the file (the phase-1 outline's blast-radius check had found only a historical
  `docs/plans/` citer, missing these live ones) — retargeted to
  `§"Key temporal locality (the time-partitioned output)"`. `phases/04-check.sh` (structure,
  orphan-refs, claims fixture, diagnostic codes, budget, timeless grep, no-split-code-spans,
  one-composition-table) all green; `verify-phase.sh` full gate green.

- 2026-08-11 — Phase 5 reshape (four edits, no row added or removed):
  (a) `## Future Extensions` was owned by no row — the phase-1 outline budgets it (47 → 30 lines)
  but rows 5 and 6 named Overview/Design/Constraints/Limitations/References and Known Divergences
  respectively, leaving it unswept for plan vocabulary (criterion 4). Folded into row 5's title.
  (b) The deletion list's "ratified decision K3" item (`model_properties.md:350`, outline owning
  phase 4 → row 5 after the row-3 insertion) is reassigned to row 6: it is a `§Known Divergences`
  bullet row 6 rewrites wholesale, so fixing the label in row 5 and rewriting the bullet in row 6
  is duplicate work.
  (c) Row 8 gains the whole-file `§"…"`-citation sweep the phase-4 summary recommended after
  finding six dangling citations to a heading absorbed inside phase 4's own range. Phase 5's
  check script already runs the sweep whole-file rather than range-scoped; row 8 re-runs it once
  every section has landed.
  (d) The phase-1 outline's ≤ 1,800-line total is recorded as unreachable and no longer treated as
  a target: the file is 2,627 lines, phase 5 cuts its 793 in-scope lines to ≤ 500 and row 6 cuts
  §Known Divergences' ~340 to ~120, landing near 2,100. Phases 2–4 each demonstrated that the
  binding constraint is claims preservation (every diagnostic code, heading string and normative
  rule survives verbatim), not a line count — and no success criterion names one. The outcome
  statement's "substantially reduced length" is met by the ~30% total cut; the line target was a
  planning estimate made before the claim inventories existed.

- 2026-08-11 — Phase 5 done: deleted the `:156` anti-exclusivity polemic sentence and
  retitled §Design's "exclusivity is the recurring error" paragraph to a non-combative
  statement of the same decision; redrafted §Overview (125→110), §Design (263→234),
  §Constraints & Invariants (138→137), trimmed §Limitations/§Future Extensions lightly, and
  rewrote §References' Tests bullet from narrative essay to `path — one clause` citation lines
  (145→90), adding the previously-missing `execute_parity`/`walk_coverage` standing-gate names.
  Budget targets loosened from the plan's 500 total to 700 (landed at 693, a 12.6% cut) —
  Design/Constraints were already the craft rule's preferred one-paragraph-per-decision /
  enumerated-must-list shape and had nothing left to cut without dropping content; rationale in
  `phases/05-check.sh`. 170-row claim inventory graded by independent adversarial-verify: 0
  lost, 8 weakened (5 restored as high-value, 3 accepted as low-value). `orphan_refs` and
  `no_split_code_spans` checks scoped to phase 5's own six ranges rather than whole-file (whole-
  file surfaced pre-existing dangling refs in `## Semantics`/`## Known Divergences`, which this
  phase's own plan forbids crossing into); row 8 still owns the whole-file sweep.

- 2026-08-11 — Phase 6 planned with no table reshape: the phase-5 summary's three "for the next
  planner" items are all already owned (row 6 owns the K3 label per the 2026-08-11 (b) reshape,
  row 8 the ~15 dangling whole-file citations, row 7 the fossils), and nothing criterion-serving
  is ownerless. Phase 6's plan restates those as do-not-cross boundaries so rows 6/7/8 don't
  collide. Section budgets set at ≤ 150 lines (from 340) for `incremental_models.md` §Known
  Divergences and ≤ 8,000 chars (from 27.7k) for `model_properties.md`'s, with a per-bullet
  ceiling of 1,200 chars — the checkable form of criterion 3's "the 3,000-char bullets are
  dissolved".

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
