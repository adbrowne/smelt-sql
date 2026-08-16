# Phase 3 plan — write addressing (repair family folded in) + the maintenance-mechanics group

## Objective

Redraft `docs/specs/incremental_models.md` §Semantics lines 752–1115 (the seven sections from
`### Per-cell write addressing` through `### The definition-change trigger`, 364 lines) into two
groups — write addressing with the repair family as a write-addressing *family* under it, and one
`### Maintenance mechanics` umbrella over the four "how a cell's technique executes" sections —
at ≤ 280 lines. Advances success criteria 3 (no restated/settled prose left in the body) and 4
(plan-vocabulary-free, drift-free spec), and delivers the §Semantics budget rows
`phases/01-outline.md` assigns to this range.

## Spec delta

No user-visible behaviour change: this is descriptive consolidation of `incremental_models.md`
§Semantics only. Target structure (all seven existing heading strings preserved **verbatim** —
`rg` shows 34/7/65/25/6/36/15 `§"…"` citations respectively across crates, sibling specs and root
`CLAUDE.md`, and root `CLAUDE.md`'s maintenance-plan-purity invariant anchors
`#statement-emission-single-owner`):

```
### Per-cell write addressing
#### The write-pattern set is open (and partly backend-provided)
#### The repair family
### Maintenance mechanics
#### Windowed maintenance and the horizon
#### Partition-local maintenance (the K8 guardrail)
#### Statement emission (single owner)
#### The definition-change trigger
```

`### Maintenance mechanics` is the one new heading (an umbrella, cited by nobody). Demotion to
`####` keeps every anchor and `§"…"` citation resolving. Rejected alternatives found in the range
move to `## Design` per the craft doc; nothing else leaves §Semantics.

## Tests

Executable checks in `phases/03-check.sh` (mirrors `02-check.sh`; run from repo root):

1. `structure` — the eight headings above appear at exactly those levels, in that order.
2. `no_orphan_refs` — each of the seven cited heading strings still exists in the spec at
   `##`–`####` level, so every external `§"…"` citation resolves.
3. `claim_inventory` — `phases/03-claims.md` exists and is non-empty (the adversarial-verify pass
   is what actually grades it).
4. `diagnostic_codes` — every diagnostic code present in the pre-edit range still appears in the
   spec body **and** in §Surface's diagnostics table (list captured verbatim in task 1;
   includes at least `MaintenanceNoAdmissibleTechnique`, `MaintenanceWriteAddressingRefused`,
   `MaintenanceWritePatternUnavailable`, `MaintenanceRepairKeysNotDiscoverable`,
   `MaintenanceRepairSliceUnbounded`, `ContractLateArrivalOutsideHorizon`,
   `MaintenanceScanUnbounded`, `MaintenanceSkeletonColumnAdded`).
5. `budget` — `### Per-cell write addressing` through the line before `### The frontier` is
   ≤ 280 lines.
6. `timeless` — `rg 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]'` over the
   redrafted range is empty.
7. `no_split_code_spans` — no line in the range ends mid inline-code/math span (the phase-2
   reflow hazard: an unmatched odd count of `` ` `` on a line inside a non-fenced paragraph).

## Tasks

1. Write `phases/03-claims.md` **before editing**: numbered inventory of every normative claim
   (must / refuses / diagnostic code / default / definition / ownership / carve-out) in lines
   752–1115 with source line ranges — parallel subagents, one per section. Record the exact
   diagnostic-code list at the top.
2. Write `phases/03-check.sh` with the seven checks; run it against the **unedited** spec to
   confirm structure/budget checks are red where expected (budget red at 364 lines).
3. Redraft `### Per-cell write addressing`: state the addressing rule once, keep the write-pattern
   pin surface and its two refusal codes, demote `#### The write-pattern set is open (and partly
   backend-provided)` and `#### The repair family` as children; the repair family's three
   fail-closed obligations (key discoverability, slice completeness, column completeness) stay
   verbatim in strength and code names, its shared preconditions cite the write-addressing rule
   rather than restating it.
4. Redraft the four maintenance-mechanics sections under `### Maintenance mechanics` with a
   one-paragraph opener naming what the group owns; cut restatements of the ladder, the contract
   lattice and per-cell admission down to `§"…"` citations (phase 2's sections now own them).
5. Move any rejected-alternative essay in the range into `## Design` as one paragraph
   (decision / rejected alternative / reason / full-path citation); do not grow §Design elsewhere.
6. Reflow prose paragraphs to the file's existing ~99–102 char width (skip fences, tables,
   headings; hanging indent for list items), then re-inspect math/inline-code spans by diff —
   check 7 automates the common case but a manual diff read is still required.
7. Run `phases/03-check.sh` → all seven green.
8. Adversarial verify: an independent subagent grades all `03-claims.md` claims against the new
   text (preserved / weakened / lost / strengthened); fix everything not `preserved` and record
   the final tally.
9. Write `phases/03-summary.md` (shipped / decisions / for the next planner / gates) and flip the
   phase-3 row to `done` in `outcome.md`.

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/03-check.sh` → all seven PASS.
- `bash .claude/scripts/verify-phase.sh` → PASS. (Set `DUCKDB_LIB_DIR`, `LD_LIBRARY_PATH`,
  `LIBRARY_PATH` to `~/.local/lib/duckdb` first — per the phase-2 summary.)
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` → empty.
- Adversarial claim verification → 100% preserved.

## Commit message

`docs(incremental-spec): redraft write addressing and maintenance mechanics`
