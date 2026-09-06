# Outcome: A source whose history is bounded and moving forward refuses or degrades, never silently under-reads

**Created:** 2026-09-06
**Status:** queued
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"Trimmed-history sources", §"Trimmed history versus SCD2 lifetime", §Open questions 3 and 4
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/incremental_models.md` §"The equivalence invariant"; `docs/specs/model_properties.md`; `docs/specs/state.md` §"The degradation contract"; `docs/specs/diagnostics.md`

## The outcome

A source can declare that its retained history is **bounded, with the bound moving
forward** as old partitions age out. smelt reasons about that bound the way it reasons
about any other model property: a model whose maintenance needs to read further back than
the source still retains is refused at analysis time with a named code, or degraded with a
recorded downgrade — never silently computed over the history that happens to survive.
Because the bound moves on its own, a model that was admissible last month can stop being
admissible with no change to the code; smelt says so when that happens instead of quietly
returning a smaller answer. The equivalence invariant's quantifier is settled explicitly
for such sources, so `full_refresh(inputs ∈ S)` has one meaning rather than two.

## Success criteria (checkable)

1. **The quantifier is decided.** `docs/specs/incremental_models.md` states whether
   `full_refresh(inputs ∈ S)` for a trimmed source is taken over *retained* history or
   over *all history that ever existed*, with the reasoning. This is the research doc's
   §Open questions 3 and it decides everything downstream — it is settled in the spec
   before any code.
2. **Declaration.** A source declares its retention bound, and the declaration says
   whether the bound is smelt-visible (declared) or merely observed (§Open questions 4 —
   settled in the decision log). Malformed forms refuse with a named `DiagnosticCode` and
   an `examples/broken/` fixture; `diagnostics_catalogue` green.
3. **Reach vs. retention, in the walk.** The comparison between a model's required
   look-back and its source's retained bound is produced by the composition walk in
   `crates/smelt-logical/src/analysis/walk.rs` — not by an ad hoc scan — per the property
   composition walk rule; `cargo test -p smelt-logical --test walk_coverage` green.
4. **Refuse or degrade, never silent.** A model whose reach exceeds the retained bound
   either refuses with a named code or takes a recorded downgrade through the existing
   degradation contract; a test covers each case, and a test asserts no path computes a
   smaller answer without one or the other.
5. **The bound moving is an event.** A source whose bound advances past what an admitted
   model needs is detected on a run and surfaced — the model's admission is re-evaluated
   against the current bound, not the bound at authoring time. A test moves the bound
   forward under a previously-admissible model and asserts the refusal or downgrade fires.
6. **Conformance.** `crates/smelt-maintenance-testkit` gains a trimmed-retention
   `SourceRecipe` whose bound advances between run steps, and
   `cargo test -p smelt-cli --test maintenance_conformance` exercises it against the
   oracle the criterion-1 decision defines. Seeded sample green.
7. **Explain and docs.** `smelt explain` renders the retained bound and the model's
   required reach against it (text and `--json`); a docs-site page documents the
   declaration, the refusal, and the degradation; `cli_docs_coverage` green.
8. **Gates green.** `bash .claude/scripts/verify-phase.sh`, `walk_coverage`,
   `statement_parity`, `execute_parity`, `maintenance_conformance`; ratchets unmoved.

## Out of scope

- The succession grain's interaction with retention (research doc tension 2 — a dimension
  outliving its source's retention). That is a genuine interaction and it is deliberately
  deferred: it cannot be designed before `20260906-scd2-keyed-succession` exists. This
  outcome must leave the quantifier decision (criterion 1) stated clearly enough for that
  work to build on, and nothing more.
- Enforcing or implementing retention — smelt reasons about a bound someone else applies.
- Backfilling history a source no longer retains.
- Any change to the contract lattice's declared points.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Settle and spec the equivalence-invariant quantifier for a trimmed source (retained history vs. all history), with reasoning — this decides the rest | pending |
| 2 | (written by phase 1's planner from the spine's requirements) | pending |

## Decision log

- 2026-09-06 (scaffold): **deliberately short**, for the same reason as
  `20260906-external-dag-steps` — the declaration's shape depends on what the spine's
  loader actually retains and on whether that retention is a smelt-visible declaration or
  an observed property (§Open questions 4). The phase-1 planner completes the table from
  the spine's findings handoff.
- 2026-09-06 (scaffold): criterion 1 is ordered first on purpose. The quantifier question
  is not a detail — under "retained history" a trimmed source is ordinary and the feature
  is small; under "all history that ever existed" every maintained model over a trimmed
  source is permanently degraded. The answer changes the size of this outcome by an order
  of magnitude, so nothing else is planned until it is written down.
- 2026-09-06 (scaffold): tension 2 (SCD2 lifetime beyond source retention) is out of scope
  here but is *not* dropped — it is named in Out of scope with the dependency that blocks
  it, so the succession work inherits it rather than losing it.

## Blocked

(none)
