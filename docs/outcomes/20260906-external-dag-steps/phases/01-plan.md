# Phase 1 — Decide the declaration shape, land the spec delta

**Outcome:** `docs/outcomes/20260906-external-dag-steps/outcome.md`
**Spec:** `docs/specs/sources.md`
**Docs:** spec-only (the docs-site page is phase 6)

## Objective

Settle the research doc's open question 2 — `produced_by:` key on a source versus a distinct
declaration kind — and write the resulting normative surface into `docs/specs/sources.md`.
Advances success criterion 1 in full, and fixes the shape every later phase implements
against (criteria 2-6). No code changes in this phase.

The decision is already recorded in the outcome's Decision log (2026-09-08): **a distinct
declaration kind**, a step YAML that names the relations it produces, because the spine's own
loader produces two sources from one invocation and a per-source key cannot express that
without a hand-synced duplicate identifier. The implementer does not re-litigate it; it writes
it up.

## Spec delta

All edits in `docs/specs/sources.md`, timeless-oracle rule (no phase vocabulary in §Surface,
§Semantics, §Design, §Constraints).

1. **§Surface, new `### Externally-produced sources (black-box steps)`** after
   `### Referential integrity`. Declares:
   - The step YAML shape and its discriminator (a top-level `external_step:` block; a file
     carrying it is a step, never a source — it declares no `columns:`), with a worked example
     modelled on `scripts/bq-dogfood-loader.sh` producing both
     `smelt.sources.raw.github_events` and `smelt.sources.raw.github_events_arrival`.
   - Keys: `produces:` (non-empty list of source addresses), `command:` (argv list),
     `description:`, and the producer's own cadence (the axis the handoff's requirement (b)
     names — enough to compare "when did data last land" against "when did the producer last
     run"). Every key's required/default/meaning row, in the same table style as
     §"Source YAML shape".
   - Addressing: a step's address is its workspace path like any node; a source is claimed by
     at most one step.
   - Explicitly **not** on the step: the produced relations' schema and world-facts. Those stay
     on the source YAML — the source declaration is the whole contract for what is produced
     (including `mutation_profile.key_recurrence`, which already carries the handoff's
     requirement (c), the redelivery arm's shape).
2. **§Surface, `### Diagnostic codes (owned by this spec)`** — new rows, names only fixed here,
   registered in `docs/specs/diagnostics.md` and the enum in phase 2:
   `MalformedExternalStep` (bad step YAML: empty/missing `produces:`, a `produces:` entry that
   is not a declared source, `columns:` alongside `external_step:`, malformed `command:`,
   malformed cadence), `SourceProducerConflict` (two steps claim one source),
   `ExternalStepNotInvocable` (a run reaches the step but may not invoke it — no command, dry
   run, or an environment that cannot), `ExternalStepFailed` (non-zero exit; names the step).
3. **§Semantics** — new numbered items: what smelt guarantees (the step is ordered ahead of
   every consumer of the relations it produces; a selected downstream pulls it in; a non-zero
   exit fails the run and leaves downstream models unbuilt; success advances the produced
   sources' frontier) and what it explicitly does not (authorship or inspection of the program,
   retries beyond the existing run policy, any idempotence guarantee, and no claim that the
   program actually wrote what the source declares — the trust rule already governs that).
   State the fail-loud rule: a run that cannot invoke the step refuses rather than reading a
   possibly-stale table.
4. **§Design** — the rejected alternative (`produced_by:` on a source) with the two reasons:
   one invocation producing several relations, and overloading a YAML grammar shared with seed
   sidecars. Note the step is a node, not a plugin/hook system.
5. **§Constraints & Invariants** — a source has at most one producing step; a step's
   `produces:` list is non-empty; smelt never authors or parses the step's program.
6. **§Known Divergences / Open Questions** — one behaviour-shaped entry: the declaration is
   specified but not yet parsed, ordered, or invoked; link this outcome.
7. **§References** — link `docs/research/20260906-bigquery-dogfood.md` §"Black-box steps in the
   DAG" and `docs/handoffs/2026-09-08-github-activity-findings.md`
   §"Requirements handed to `20260906-external-dag-steps`".

## Tests

No red-green code tests — this phase writes spec prose only. The checks that must hold:

- `no_phase_vocabulary_in_sources_spec` — grep: zero `Phase [A-Z0-9]` matches in the new
  §Surface/§Semantics/§Design/§Constraints text (the timeless-oracle rule; `/smelt:validate`
  flags these).
- `diagnostics_spec_unchanged` — grep: the four new code names appear in `sources.md` only;
  `docs/specs/diagnostics.md` is untouched this phase, so `diagnostics_catalogue` cannot go red
  on a code with no enum variant.

## Tasks

1. Re-read `docs/specs/sources.md` §Surface, §Semantics, §Design, §Constraints for house style.
2. Write the new §Surface section (item 1) with the worked two-relation example.
3. Add the four diagnostic-code rows (item 2).
4. Add the §Semantics guarantees/non-guarantees items (item 3).
5. Add the §Design rationale, §Constraints items, §Known Divergences entry, §References links
   (items 4-7).
6. Run the two greps above; run the verification gates.

## Verification

```bash
bash .claude/scripts/verify-phase.sh
rg -n 'Phase [A-Z0-9]' docs/specs/sources.md            # expect no hits in spec body
git diff --stat                                          # expect docs/specs/sources.md only
bash .claude/scripts/large-file-check.sh
```

## Commit message

`spec(sources): declare externally-produced sources as black-box DAG steps`
