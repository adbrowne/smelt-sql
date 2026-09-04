# Outcome: Walk-migration residue — the composition walk is the sole source of every property

**Created:** 2026-09-04
**Status:** queued
**Source:** `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md` rows MP-03, MP-05, MP-11, MP-13 (each classified "migration backlog, not a design question"); `docs/specs/model_properties.md` §Known Divergences
**Spec anchors:** `docs/specs/model_properties.md` §Constraints "Composition happens in the walk, not in scans", `docs/specs/architecture.md` §"Property composition walk rule"

## The outcome

Every composition-relevant model-property verdict comes from the shared bottom-up walk in
`smelt-logical`'s `analysis/walk.rs`. Scopes inside expression-position subqueries are walk
nodes. The cumulative classifier's whole-SQL `OVER(` check is either a walk-invoked leaf
classifier or gone. Every maintenance-cell route that can consult a declared referential-integrity
closure does so, not only the source-enrichment route. The `walk_coverage` gate, not a doc comment, is what
says the rule holds.

## Success criteria (checkable)

1. Expression-position (scalar and `EXISTS`) subqueries and redundantly-parenthesised derived
   tables are enumerated as walk nodes; a property test shows a bound/reach/grain verdict for a
   model reading such a scope equals the verdict for the same model with the scope inlined.
2. `classify_cumulative`'s `OVER(`/`OVER (` text check is classified onto the walk as a leaf
   classifier over one bounded node's text, or deleted; no whole-SQL scan remains in
   `rules/cumulative.rs` (grep-asserted by `walk_coverage`).
3. Every maintenance-cell admission route that takes a `JoinContext` receives the declared-RI
   closure map (none passes an empty map); a fixture that admits only with the closure present
   exists per route.
4. `model_properties.md` §Known Divergences bullets for MP-03, MP-05, MP-11 and MP-13 are
   deleted; `/smelt:validate model_properties` clean.
5. `cargo test -p smelt-logical --test walk_coverage`, `maintenance_conformance`,
   `statement_parity` and `verify-phase.sh` green; no new whole-text scan introduced.

## Out of scope

- Merging the `EffectiveWindow` and `BoundResult` walks (MP-02, an architecture decision).
- Anything to do with declared lateness. Decided 2026-09-04 that lateness is orchestration-only
  and never a plan or probe input (`docs/research/20260904-decision-track.md`); the former
  criterion 4 (probe consults lateness) was removed for that reason, and the probe's
  late-append classification is `docs/outcomes/20260904-decision-residue/outcome.md`'s.
- Widening skeleton-source closure beyond non-aggregating scopes (MP-10, admission width).
- `SourceUniqueKeyViolated`'s missing emitter (MP-14, undecided).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Expression-position subqueries and parenthesised derived tables as walk nodes; inline-equivalence property test | pending |
| 2 | Cumulative classifier: `OVER(` check onto the walk as a leaf classifier or deleted; `walk_coverage` asserts it | pending |
| 3 | Declared-RI closure reaches every `JoinContext`-taking maintenance-cell route; per-route fixtures | pending |
| 4 | Delete the four divergence bullets; `/smelt:validate model_properties`; all gates green | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
