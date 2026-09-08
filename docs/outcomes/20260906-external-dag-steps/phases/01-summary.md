# Phase 1 summary — Decide the declaration shape, land the spec delta

**Shipped:**
- `docs/specs/sources.md` gains the full normative surface for external steps: §Surface
  `### Externally-produced sources (black-box steps)` (declaration shape, worked two-relation
  example modelled on `scripts/bq-dogfood-loader.sh`, key table for `produces:`/`command:`/
  `cadence:`), four new diagnostic rows in the existing `### Diagnostic codes` table
  (`MalformedExternalStep`, `SourceProducerConflict`, `ExternalStepNotInvocable`,
  `ExternalStepFailed`), six new §Semantics items (ordering, frontier advancement, failure
  propagation, invocation refusal, non-guarantees), a §Design paragraph for the rejected
  `produced_by:`-on-a-source alternative, three §Constraints items, one §Known Divergences
  entry (nothing parsed/ordered/invoked yet), and §References links to the research doc and
  the findings handoff.
- No code changes — this phase is spec prose only, per the plan.

**Decisions:**
- The distinct-declaration-kind decision was already recorded in the outcome's decision log
  (2026-09-08, phase 1 planning) before this phase started; this phase writes it up in
  §Design rather than re-deciding it. Reasoning restated in the spec: one invocation can
  produce several relations, and a per-source key would overload the seed-sidecar-shared
  grammar.
- The step's discriminator is checked before the source/seed-sidecar tiebreaker: a file
  carrying `external_step:` is a step regardless of a sibling `.csv`, never a source.
- `cadence:` (producer's own schedule) is kept explicitly distinct from
  `mutation_profile.lateness` (clock-relative arrival lag) — they answer different staleness
  questions, per handoff requirement (b).

**For the next planner:**
- Phase 2 (parse/validate in `smelt-core`) has everything it needs: four diagnostic code
  names, the exact key table, and the discriminator rule. `examples/broken/` fixtures for
  each malformed form still need to be created.
- The spec deliberately does not specify cross-referencing `produces:` addresses against
  actually-declared sources at parse time vs. at a later resolution pass — phase 2's plan
  should decide where that check lives (likely parse-time, matching `MalformedSource`'s own
  posture for similar cross-references, e.g. `referential_integrity` subset checks).
- `command:` argv supports a `{run_date}`-style placeholder in the worked example; the spec
  doesn't yet normatively define a placeholder-substitution grammar for `command:` entries —
  worth deciding explicitly in phase 4 (invocation) rather than assuming the example's shape
  is load-bearing.
- Nothing here touches `docs/specs/diagnostics.md`; phase 2's plan must add the catalogue
  rows there (verified by `diagnostics_catalogue`) since this phase deliberately left it
  untouched (test `diagnostics_spec_unchanged`).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full
  workspace test, example_diagnostics).
- `rg -n 'Phase [A-Z0-9]' docs/specs/sources.md` — one hit, inside the pre-existing
  Timeless-oracle rule's own explanatory text (not new content) — no phase vocabulary
  introduced by this phase's edits.
- `git diff --stat` — `docs/specs/sources.md` only.
- `bash .claude/scripts/large-file-check.sh` — PASS.
