# Phase 1 outline — blueprint for phases 2–7

This is the artifact phases 2–7 execute against (`docs/outcomes/20260809-incremental-spec-redraft/
phases/01-plan.md` task 5). It contains three things: the target section outline for
`incremental_models.md`, the terminology table, and the ratified deletion list.

## Target outline

Current `incremental_models.md` is 3,017 lines. Target total ≤ 1,800 lines. Budgets are per top-
level section (and, for `## Semantics`, per major subsection group); they are planning targets
for the phases below, not hard per-section limits enforced by this phase.

| Section | Intent | Existing sections merged/demoted/deleted | Budget |
|---|---|---|---|
| `## Overview` | Unchanged shape: the one guarantee, the two declared facts, the four corners, the plan, running example, reading guide. Trim "Why cells differ" to its cost-summary paragraph; the write-cost verb/addressing detail moves to §"Per-cell write addressing" (no new content, just de-duplicated). | Kept, trimmed. | 110 |
| `## Surface` | Declared shape, `maintenance:`/`contract:` overrides, partition/key-grain frontmatter, column-family catalogue, CLI, diagnostics. | Kept; `nondeterministic_columns`/`batched.*` rows removed by phase 6 (deletion list below), not here. | 260 |
| `## Semantics` | The spec's core. Reorganised around typed deltas and the lattice (phase 2), the frontier (landed phase 1), the graph layer, and the two shape profiles collapsed around one composed key+time composition table (phase 3). | See subsection table below. | 1020 |
| `## Design` | One paragraph per decision + rejected alternative + citation. Anti-exclusivity polemic (below) deleted; every remaining paragraph re-checked against the craft doc's "one paragraph per decision" rule. | Kept, halved — currently carries restated Semantics content this redraft moves up. | 90 |
| `## Constraints & Invariants` | Enumerated musts, per shape. | Kept, trimmed of restated Semantics prose. | 70 |
| `## Limitations` | SCD2 boundaries, other deliberate boundaries. | Kept. | 40 |
| `## Known Divergences / Open Questions` | Gap-first list per the craft doc; landed-work narratives dissolved (phase 5). | Rewritten wholesale — see deletion list. | 120 |
| `## Future Extensions` | Kept. | Kept. | 30 |
| `## References` | Plans/research citations; phase-vocabulary in surrounding prose removed. | Kept, trimmed. | 60 |
| **Total** | | | **1,800** |

### `## Semantics` subsection budget

| Subsection | Intent | Existing sections folded in | Budget |
|---|---|---|---|
| Typed deltas & the algebraic ladder | One combined introduction: the equivalence invariant, the algebraic maintenance ladder, decomposed state, "validator not chooser" — all facts about what a column's combiner algebra licenses, stated once instead of three adjacent sections restating the ladder. | §"The equivalence invariant", §"The algebraic maintenance ladder", §"Decomposed state (rung 2) in keyed models", §"Validator, not chooser" | 140 |
| The contract lattice | Unchanged home; cross-references typed deltas instead of restating them. | §"The contract lattice" | 65 |
| The plan matrix & per-cell admission | Combined: the plan matrix's per-input dispatch and per-cell admission's three obligations are one dispatch-then-admit story. | §"The plan matrix", §"Per-cell admission" | 90 |
| Per-cell write addressing | Write addressing, the open write-pattern set, and the repair family (a per-group recompute is a write-addressing family, not a separate concept) combined. | §"Per-cell write addressing", §"The write-pattern set is open…", §"The repair family" | 140 |
| Maintenance mechanics | Windowed maintenance & the horizon, the K8 partition-local guardrail, statement emission, the definition-change trigger — all "how a cell's technique actually executes" facts. | §"Windowed maintenance and the horizon", §"Partition-local maintenance (the K8 guardrail)", §"Statement emission (single owner)", §"The definition-change trigger" | 140 |
| The frontier | Landed this phase. | §"The frontier" (+ two realization subsections) | 30 |
| The graph layer | Kept near-unchanged — already dense and non-duplicative. | §"The graph layer" | 110 |
| Shape profiles (intro) | The composition-table framing paragraph. | §"Shape profiles" | 15 |
| The partition grain profile | Composition table + local machinery only; execution-model/strategy-enum detail trimmed to what is not derivable from the emitters themselves. | §"The partition grain" and its subsections | 130 |
| The key grain profile | Composition table + local machinery; the composed key+time corner (phase 3) replaces "Key temporal locality" + "What the composed shape enables" + "Key-grain output shape" with one shared table instead of three prose sections making overlapping claims. | §"The key grain" and its subsections | 150 |
| Interactions | Kept as a short cross-shape note. | §"Interactions" | 10 |
| **Subtotal** | | | **1,020** |

## Terminology table

| Old term(s) | Redraft term | Defined once at |
|---|---|---|
| "the reconciliation ledger" and "the transactional merge ledger" treated as two concepts; occasional "interval ledger" conflation | **frontier** (realizations: **frontier record**, **transactional frontier write**) | `incremental_models.md` §"The frontier" |
| "output-delta shape" / `OutputDelta` enum / "delta shape" / ad hoc "dirt" | **typed delta** | `model_properties.md` §"Output-delta shape" |
| "cell" (used consistently already, no drift) | **plan cell** | `incremental_models.md` §"How smelt maintains it — the plan" |
| "verb" (`INSERT`/`DELETE`/`UPDATE`/`MERGE`, used consistently already) | **verb** | `incremental_models.md` §"Why cells differ — the three costs" |
| "lattice point" / "relaxation" | **contract point** | `incremental_models.md` §"The contract lattice" |
| "profile" / "shape" used interchangeably for partition-grain vs. key-grain | **shape profile** | `incremental_models.md` §"Shape profiles" |

The first two rows are genuine renames this outcome performs (frontier unification landed phase
1; typed-delta terminology adopted in phase 2). The last four rows are already-consistent terms
in the current spec — listed so phases 2–7 have one place to check they haven't drifted, not
because they need renaming.

## Ratified deletion list

Every anchor below was re-confirmed by `rg` in this phase (commands and counts recorded in
`phases/01-summary.md`). An anchor `rg` could not confirm is marked "not present — no work"
rather than silently dropped.

| Item | Anchors | Owning phase | Disposition |
|---|---|---|---|
| Anti-exclusivity polemic ("Text anywhere in this corpus that treats 'partitioned' and 'keyed' as mutually exclusive alternatives is wrong and is corrected against this section.") | `incremental_models.md:156` | 4 | delete — the composed-shape fact stands on its own; the corrective aside about other text being "wrong" is polemic, not a spec claim. |
| Anti-exclusivity polemic, second instance ("The axes compose; exclusivity is the recurring error.") | `incremental_models.md:2053` (§Design) | 4 | delete — same fact as above, restated combatively; the non-polemic content (axes compose) survives as ordinary Design prose. |
| Dead `IncrementalStrategy` variants `Append`, `InsertOverwrite` | `incremental_models.md:1438-1441` (enum definition), `:1450` (`Append` unreachable note), `:2187-2188` (Design restatement), `:2343` (Constraints restatement), `:2597` (Known Divergences: `InsertOverwrite` dead-code note) | 6 | delete — DuckDB always uses `DeleteInsert` (stated at the enum site); the two unreachable variants and their three restatements collapse to one sentence noting only `DeleteInsert` is live, with the backend capability that would admit the others named as future work if any survives triage. |
| `batched.*` config fossils — `.sql` frontmatter sub-block | `models.md:244`, `models.md:249` (already a hard error; `nondeterministic_columns`/`unique_key`/`safety_overrides` retired with fix-its) | 6 | no work — already fail-loud; kept as-is, just swept for phase-vocabulary in the redraft's own cross-references. |
| `batched.*` config fossil — `smelt.yml` model override sub-block (`models.<name>.batched.unique_key`) | `incremental_models.md:2687`, `models.md:249` ("a separate parsing path this retirement does not reach"), `models.md:342` | 6 | delete — retire the `smelt.yml`-override `batched:` sub-block with the same fail-loud fix-it pattern already used for the `.sql` frontmatter form; the MERGE-dedup-only `unique_key` it carries needs a named top-level replacement decided as part of phase 6's spec delta. |
| `nondeterministic_columns` list-form declaration (parser-level) | `model_properties.md:67`, `:100`, `:140`, `:310`, `:350` (**superseded**, "not yet removed from the parser") | 6 | delete — remove the list-form parse path from `smelt-core`; `columns.<c>.contract: plausible` is the sole surface, with a fail-loud diagnostic naming the replacement for any caller still writing the old key. |
| `grain: key_per_partition` | `models.md:75`, `:129`, `:131`, `:133`, `:342`; `incremental_models.md:53`, `:151`, `:158`, `:300`, `:2651` ("derives no plan"); `cli.md:246`; `diagnostics.md:499` (`MaintenanceUnsupportedGrain`) | 6 | delete from the declared surface — the value parses and validates today but has no execution path (§Known Divergences), and implementing one is new behaviour out of this outcome's scope (§Out of scope). Retire the grain value with a fail-loud diagnostic naming the two facts (`timeseries:` clock + `partition_column ∈ unique_key`) that would have derived it, pointing at `grain: key` as the closest supported shape. |
| "Seven proofs" phrasing | `model_properties.md:325` ("All seven maintenance-plan proofs are built…"), `incremental_models.md:2599` ("All seven maintenance-plan proofs are derived…") | 5 | rewrite — both are Known-Divergences entries narrating a completed proof count; rewritten gap-first (only the "one residue survives" content is a live gap) with the "seven" enumeration dropped, since a reader six months out cannot check the count against anything. |
| Settled-decision essays in `incremental_models.md` §Known Divergences | The `deferral` scheduling entry (§"The contract, plan, and graph layer", first bullet) mixes ~15 lines of landed-work narrative ahead of the one live gap (now partly addressed: frontier vocabulary landed phase 1, narrative trim still owed) | 5 | rewrite — trim to gap-first form per the craft doc; landed-work detail either deletes or promotes to §Design if it explains a rejected alternative. |
| Settled-decision essays in `model_properties.md` §Known Divergences | Proof-layer residues entry (`:325`, spans into skeleton-closure/fingerprint-projection entries `:328-329`) | 5 | rewrite — same gap-first trim; the "how it's built" implementation narrative is not a divergence from the spec, only the residual gaps are. |
| "Ratified" / internal decision-label process vocabulary | `model_properties.md:350` ("ratified decision K3") | 4 | rewrite — cite the research doc by full path only, per the craft doc's "Research shorthand" rule; drop the internal label. |

Not present — no work: no `Phase [A-Z0-9]`-style plan-phase heading tags were found in
`incremental_models.md` or `model_properties.md` body sections (one pre-existing citation,
`incremental_models.md` "Tracked: `docs/plans/20260809-sensitivity-precision.md` Phase 6.", was
fixed in this phase — see `phases/01-summary.md`); no `(Historical name: …)` / "pre-cut" /
"category error" occurrences were found in either file.
