# Phase 2 plan — the contract + plan-matrix core, redrafted around typed deltas

## Objective

Redraft `docs/specs/incremental_models.md` §Semantics from its opening paragraph through
§"Per-cell admission" (currently lines 448–833, 386 lines) into three grouped subsections
totalling ≤ 300 lines, per `phases/01-outline.md`'s Semantics budget (140 + 65 + 90). The
redraft adopts **typed delta** as the term of art (terminology-table row 2, the one genuine
rename this phase performs) and drops the restatements that make the invariant, the ladder and
the validator rule each say the same thing three times. Advances criteria 3–4 (no landed-work
narrative or plan vocabulary in the redrafted text) and the outcome statement's length target;
touches no behaviour, so no docs-site or code change lands here.

## Spec delta

Descriptive only — no user-visible behaviour changes, so the spec edit *is* the phase.
`docs/specs/incremental_models.md` §Semantics gets this structure (all existing heading strings
preserved; the only new heading is the group intro, the rest are **demoted from `###` to
`####`**, never renamed — ~100 `§"…"` citations across sibling specs, root `CLAUDE.md` and six
crates depend on the exact strings):

```
### Typed deltas and the algebraic ladder      (new group heading + ~15-line intro)
#### The equivalence invariant
#### The algebraic maintenance ladder
#### Decomposed state (rung 2) in keyed models
#### Validator, not chooser
### The contract lattice                        (unchanged level; cross-references, not restates)
### The plan matrix
#### Per-cell admission
```

Content rules for the redraft:

- The group intro defines **typed delta** once by citing its owner (`model_properties.md`
  §"Output-delta shape") and states the one fact the four children share: what a column's
  combiner algebra licenses. Ad hoc "delta shape" / "dirt" phrasings inside these lines become
  "typed delta".
- The invariant is stated normatively once; the ladder's "holds unconditionally on every rung"
  and the admission section's `S`-indexed refinement reference it instead of re-deriving it.
- §"Validator, not chooser" keeps its rule verbatim in strength; §"Per-cell admission"'s
  closing interchangeability paragraph cites it rather than restating the rule.
- §"Decomposed state" keeps the state-shape catalogue table, the collision refusal
  (`KeyedStateColumnCollision`) and the presentation-projection rules at full strength; its
  *rejected alternative* prose (the separate `<model>__state` table + view) moves to §Design as
  one paragraph, which is where the craft doc puts rejected alternatives.
- §"The contract lattice" keeps the triple rule and both points' oracles/probes/diagnostic codes
  (`ContractLateArrivalOutsideHorizon`, `ContractDeferralExceeded`) verbatim in strength; the
  deferral point's third paragraph (scheduling mechanics: `skipped_deferral`,
  `skipped_deferral_upstream`, subsumption proof) is compressed to its normative musts, with the
  manifest-recording surface referenced from its owning spec rather than re-narrated.
- Every rule's exact strength word (`never`, `refused`, `fail-loud`, `hard error`) is preserved;
  shortening comes only from dropping restatements, per `docs/specs/CLAUDE.md` §Calibration.

## Tests

This phase edits prose, so the red-green tests are executable greps/lints, added as a shell gate
`docs/outcomes/20260809-incremental-spec-redraft/phases/02-check.sh` (run by hand; not a cargo
test — no code changes ship here):

1. `structure` — the seven headings above exist at exactly the listed levels in the listed order.
   Red before the edit (group heading absent, children at `###`).
2. `no_orphan_refs` — every `§"…"` string cited anywhere outside `docs/plans/` and
   `docs/research/` that names one of the seven headings still resolves to a heading in
   `incremental_models.md`. Red if any demotion accidentally renames.
3. `claim_inventory` — every normative claim inventoried from the pre-edit 448–833 range (task 1)
   appears in the post-edit text; the inventory file itself is the fixture. Red until the
   redraft covers all of them.
4. `diagnostic_codes` — the four codes named in the range (`KeyedStateColumnCollision`,
   `ContractLateArrivalOutsideHorizon`, `ContractDeferralExceeded`,
   `MaintenanceRepairKeysNotDiscoverable`) still appear, and each still appears in §Surface's
   diagnostics table.
5. `budget` — the redrafted range is ≤ 300 lines.
6. `timeless` — `rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]'` over the
   redrafted range is empty.

## Tasks

1. Inventory every normative claim in lines 448–833 (must/refuse/diagnostic/default/definition/
   ownership/carve-out) into `phases/02-claims.md`, numbered with source line ranges — the
   claim-inventory method, `docs/specs/CLAUDE.md` §"Large redrafts". Do this **before** editing.
2. Write `phases/02-check.sh` with the six checks above; run it and record the red output.
3. Redraft the group: new `### Typed deltas and the algebraic ladder` intro, demote the four
   children to `####`, dedup the invariant/ladder/validator restatements.
4. Move §"Decomposed state"'s rejected-alternative paragraph into §Design (one paragraph,
   research citation by full path).
5. Trim §"The contract lattice" per the content rules; verify both oracles and both probe
   descriptions survive verbatim in strength.
6. Demote `Per-cell admission` to `####` under §"The plan matrix"; fold its interchangeability
   paragraph's validator restatement into a citation.
7. Adversarially verify: an independent reviewer grades every claim in `02-claims.md` against the
   new text as preserved / weakened / lost / strengthened; fix everything not `preserved`.
8. Run `02-check.sh` green, then the full verification below; write `phases/02-summary.md`.

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/02-check.sh` → all six green.
- `bash .claude/scripts/verify-phase.sh` (full, not `--fast`).
- `rg -n '§"' docs/specs/incremental_models.md | wc -l` cross-checked against a resolve-all pass
  (check 2) — no dangling internal section reference.
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` → empty.

## Commit message

`docs(incremental-spec): redraft the typed-delta, lattice and plan-matrix core`
