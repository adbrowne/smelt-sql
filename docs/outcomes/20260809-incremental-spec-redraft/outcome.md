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
| 3 | Redraft write addressing (repair family folded in) and the maintenance-mechanics subsections | pending |
| 4 | Redraft the shape profiles; state the composed key+time corner's single composition table | pending |
| 5 | Overview / Design / Constraints / Limitations / References pass: polemics and plan-vocabulary deleted, terminology aligned | pending |
| 6 | Rewrite Known Divergences (both specs) as genuine gap lists | pending |
| 7 | Remove parser/config fossils with fail-loud diagnostics | pending |
| 8 | docs-site terminology sync; validate + timeless greps clean | pending |

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

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
