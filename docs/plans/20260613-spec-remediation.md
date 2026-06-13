# Plan: Spec-corpus remediation (2026-06-12 review)

**Date**: 2026-06-13
**Source review**: [`docs/research/20260612-spec-review.md`](../research/20260612-spec-review.md) — 282 findings (26 Critical · 117 Major · 131 Minor · 8 Nit) across all 30 specs, adversarially verified.
**Spec**: corpus-wide (all of `docs/specs/`); two specs spin out their own `/smelt:spec` cycles.
**Docs**: docs-only for this plan (spec + `docs-site/` edits). Code rollout is **planned but spawned** as separate `/smelt:plan` cycles (see Track D).
**Tracking branch**: `worktree-spec_review` (this worktree) → PR to `main`.
**Precedent**: mirrors [`docs/plans/20260504-spec-review-followup.md`](20260504-spec-review-followup.md) (docs-only, batched-by-spec, progress table, spawn-to-followup).

---

## Context

A full multi-agent review of the spec corpus landed in `docs/research/20260612-spec-review.md`. It is **spec-text-only** (no code consulted; implementation drift is `/smelt:validate`'s job). Every finding already carries the two axes this plan triages on:

- **dimension** — solution-quality / clarity / correctness-consistency (not load-bearing for sequencing).
- **implication** — the sequencing axis:
  - `spec-edit-only` (48 C/M + most Minors) — wording fix, the `_Fix:_` line is directly applicable. **No-brainers.**
  - `design-decision-needed` (68 C/M + some Minors) — the design needs a human call before the spec can be edited.
  - `implementation-impact` (31 C/M) — if the spec is right, the code likely drifted and needs a follow-up code change.

The problem this plan solves: 282 findings is too many to action interactively without burning tokens, and the design-decision findings can't be auto-fixed. The strategy is **separate the mechanical from the judgment**: sweep the systemic/no-brainer edits first (no human needed), then collapse all 68+ design calls into **one markup document the user fills in once** (replacing ~68 interactive question rounds), then apply decisions and roll code out in **subsystem-grouped waves** rather than spec-by-spec churn.

Intended outcome: the corpus becomes internally consistent and contradiction-free (specs are the 1000×-leverage artifact per CLAUDE.md), and the implementation-impact backlog is organized so code rollout touches each crate cluster once.

---

## Triage model (how findings map to tracks)

| Track | What | Findings | Human input | Output |
|-------|------|----------|-------------|--------|
| **A — Systemic sweeps** | Corpus-wide mechanical fixes flagged by many units | ~5 sweep classes | none | spec edits |
| **B — Per-spec no-brainer batches** | `spec-edit-only` `_Fix:_` lines, internal to one spec | 48 C/M + Minors/Nits | none | spec edits |
| **C — Design decisions** | `design-decision-needed` findings | 68 C/M + Minor | **one markup doc** | decision log → spec edits |
| **D — Code rollout** | `implementation-impact` findings, after specs settle | 31 C/M | per-wave review | spawned `/smelt:plan` cycles |
| **R — Rewrites** | The two genuine design holes | cumulative + incremental | `/smelt:spec` | new spec versions → feeds D |

**Prioritisation principle** (severity × leverage): do the cheap corpus-wide sweeps first (Track A — kills whole defect *classes* in one commit), then settle decisions (Track C — unblocks everything cross-spec), then the per-spec no-brainers (Track B), then rewrites (R) and code (D). Critical findings inside each track go first.

---

## Session map (resumable; each session = small, bounded context)

Each session loads **only**: this plan + the named spec(s) + the cited review-doc line range. Never re-read the whole 265 KB review. After each session, update the Progress tracking table and commit.

### Session 1 — Track A: systemic sweeps (no decisions)
Five corpus-wide passes, **one commit each**. These were independently flagged by 6+ review units — fixing once removes a defect class corpus-wide.

- **A1 — "owned here until `diagnostics.md` lands" dead conditional.** `diagnostics.md` has landed. Find-delete the clause and re-point ownership to it. Specs: `functions.md`, `planner_integration.md`, `scoping.md`, `types.md`, `gradual_typing.md` (+ any other hit). Verify: `rg -l 'until a .diagnostics\.md. spec lands' docs/specs` returns empty.
- **A2 — diagnostic call-path spelling sweep.** Retire `smelt.fn.<path>` and `smelt.functions.<name>`; standardise on the `smelt.<path>(…)` form architecture.md §Resolution mandates. Specs: `diagnostics.md`, `lsp.md`, others. Verify: `rg 'smelt\.fn\.' docs/specs` empty; `smelt.functions.` only where intentional.
- **A3 — `workspace` → `project` vocabulary sweep.** Project-isolation is a `stable` invariant; pre-invariant specs still say "workspace". Specs: `diagnostics.md` (UnknownSmeltFn/SmeltRecordRedefinition scope), `lsp.md` (hover/completion counts), `architecture.md`. Reconcile each "workspace" against the project-isolation rule; change where it means project-scope.
- **A4 — SPEC_TEMPLATE conformance sweep.** Produce a single conformance table across all 30 specs (replacing the ad-hoc per-spec "missing scope callout" findings). Fix: add `diagnostics.md`'s missing scope-callout blockquote; standardise `**What this is.**` label (16 vs 12 split); decide flat-bullet vs nested `### Code`/`### Tests` References (17 specs — this is one `design-decision-needed`, see Track C if non-obvious); fix the two timeless-oracle borderline cases (`planner_integration.md` "Phase 42", `seeds.md` "as of Phase 4"). Review-doc refs: §"Mechanical / template-conformance" + §"Completeness critic".
- **A5 — diagnostics registry reconciliation (catalogue-level).** The codes-referenced-but-absent and owner/consumer attribution conflicts: `ColumnTypeUnresolved` status, `UnknownColumn`→`UndeclaredColumn`, `IncrementalNotBatchSafe` trigger, cumulative-codes owner header, planner-validation-codes owner. Several are `spec-edit-only`; the *does-this-code-exist-and-fire* ones are `design-decision-needed` → carry those into Track C, do the rest here.

*Output of Session 1: ~5 commits, plus the SPEC_TEMPLATE conformance table written into this plan's appendix.*

### Session 2 — Track C prep: generate the decision document
Author `docs/research/20260613-spec-remediation-decisions.md` — **the markup doc**. No spec edits this session. One generation pass over the 68 `design-decision-needed` findings (+ design-flavoured Minors), grouped into the themes below. Each entry:

```
### D-NN  <finding ref (C7 / Major / Minor) · spec.md §section>
Conflict: <1–2 lines>
Options:  A) … (recommended)  B) …  [C) …]
Recommendation: A — <one-line why>
Decision: ____________________   ← user fills this
Notes:    ____________________
```

**Decision themes** (review-doc refs in parentheses):
1. **Architecture resolution & naming** — C1 resolution table, C2 non-injective name mapping / emitted-name collision diagnostic, scan-universe definition, schema `main` default vs required, C22 `smelt-logical` ownership.
2. **Diagnostics existence/severity** — which codes exist & fire (`ColumnTypeUnresolved`, `UnknownSmeltPath`), lsp↔diagnostics severity disagreements, code-owner attribution, References flat-vs-nested (A4 spillover).
3. **Meta-language reflection & precedence** — C11 spread/pipe precedence, C12 ModelRef.name literal-vs-identifier, C13 ordering tiebreak, ternary/pipe precedence, ColumnRef.type comparison, HOF named-arg code.
4. **Python models ↔ meta-language** — C14 `--- name ---` delimiter clash, C15 circular-dep vs iterative generation, evaluation-order reconciliation, reflection-API divergence (find_models vs with_tag), name/path/directory identity.
5. **Type system** — C16 decimal widening soundness (also schema_evolution), C17 fragment-kind direction, C26 nullability-in-signatures (gradual_typing vs types §11), VALUES temporal LUB, decimal-arithmetic trigger.
6. **Project surface / config** — `format` model-vs-target precedence, `default_materialization` accepting `test`/`cumulative`, orphaned `vars:`/`state:` keys, source `name:` `<schema>.<table>` multi-target.
7. **CLI & selection** — canonical-display round-trip vs no-prefix, selector `+`/`path:` expansion, not-found-vs-no-op + exit codes, `--exclude +model` inconsistent set, scope fall-through stability.
8. **Per-spec smaller design calls** — datagen scale-invariance & FK bounds, seeds compile/runtime divergence framing, testing input-key form (C18) & empty-CTE foot-gun & DECIMAL compare, schema_evolution NOT-NULL reclassification & nested narrowing, virtual_environments reuse hatches (accept_current vs assert_deterministic) & posture lattice, expansion Caller-tag identity, lsp watched-set & cross-file republication & rename scope, data_catalog lineage/exclusion shapes, run_state sub-day granularity, timeseries nullability & granularity-vs-partition-type, planner_integration cardinality enum & monotonicity wording.

*Hand the doc to the user. User marks it up offline — this is the token-saving step. The marked-up doc is the persistent decision log for Sessions 4+.*

### Session 3 — Track B: per-spec no-brainer batches
Apply `spec-edit-only` `_Fix:_` lines, **one commit per spec**, only for fixes that are internal to one spec and don't depend on a Track-C decision. Order by score (worst first): `cumulative_aggregate` (defer — rewrite), `lsp`, `diagnostics`, `meta_language`, then the rest. Cross-spec `spec-edit-only` fixes whose authoritative side is already settled (e.g. C24 cli seed-step → seeds.md; cumulative `--start/--end` → incremental flags; partition-column projection dedup → timeseries) land here too. Bump each spec's `last_reviewed` to the edit date.

### Sessions 4–N — Track C execution: apply decisions
Once the markup doc is returned, apply each decided fix to the specs, **one commit per theme** (themes 1–8 above). Where a decision implies a `docs-site/` user-doc change, make it in the same commit (timeless-oracle rule). Update `last_reviewed`.

### Sessions R1/R2 — the two rewrites
- **R1 — `cumulative_aggregate.md` rewrite** (`/smelt:spec cumulative_aggregate`). Brainstorm + spec the missing merged-partition state/ledger, retry-after-partial-failure, NULL-aware combine, classifier gaps (HAVING/DISTINCT/LIMIT/set-ops), `state.mode` dependency. Closes C3, C4, C25 and the cumulative Major cluster. Feeds Track D.
- **R2 — `incremental_models.md` rewrite** (`/smelt:spec incremental_models`). Brainstorm + spec the write-window-vs-run-window single source of truth, write-skew bound derivation, chained-session classification, unified window-admission rule, strategy-choice correctness constraint. Closes C7, C8, C9, C10 and the incremental Major cluster. Feeds Track D.

### Sessions D1+ — Track D: grouped code rollout (spawned)
**Group by subsystem, not by spec**, so each crate cluster is touched once (per the user's churn-avoidance directive). Each wave: `/smelt:validate` the relevant specs *together* → read the combined drift report → one `/smelt:plan` → `/smelt:implement`. Waves:

- **D-diag** — diagnostics & LSP code surface: DiagnosticCode enum additions/renames, severity map, project-scoped resolution. Specs: diagnostics, lsp, scoping. Crates: `smelt-db` (DiagnosticCode), `smelt-lsp`.
- **D-resolve** — addressing/resolution: emitted-name collision diagnostic (C2), resolution-table behaviour (C1), scan universe. Crates: `smelt-core`/`smelt-db` resolver.
- **D-types** — type soundness: decimal widening (C16, schema_evolution), fragment-kind ceiling (C17), nullability origins. Crates: `smelt-types`, `smelt-db/type_inference`.
- **D-incr** — incremental + cumulative analysis: follows R1/R2; write-window/classifier code. Crates: `smelt-planner/rules`.
- **D-cli** — selector/exit-code/no-op behaviour. Crates: `smelt-cli`.
- **D-misc** — function_schema_inference (bare-`*`), data_catalog (path/edges), datagen, output_fingerprint folding. Per-crate.

Each wave is its own `docs/plans/YYYYMMDD-<wave>.md`, **not authored here** — listed in the spawn table so they're visible and ordered.

### Final session — ROADMAP + close-out
- Add the three deferred review passes to `docs/ROADMAP.md` (incremental/cumulative × machine-generated bodies; identifier-quoting/codegen-safety; backend-matrix consistency) as 🔮 future work with rationale.
- Confirm Progress tracking table fully `done`; open the PR.

---

## Progress tracking

Update after every session. Finding IDs reference `docs/research/20260612-spec-review.md`.

### Track A — systemic sweeps
| Sweep | Status | Specs touched | Commit | Date |
|-------|--------|---------------|--------|------|
| A1 owned-until-lands | done | functions, gradual_typing, incremental_models, planner_integration, scoping, types | c7704007 | 2026-06-13 |
| A2 call-path spelling | done | diagnostics, lsp | 1932a62a | 2026-06-13 |
| A3 workspace→project | done | diagnostics, lsp | 23f9511a | 2026-06-13 |
| A4 template conformance | done | 25 specs (all but function_schema_inference, output_fingerprint, run_state, virtual_environments — already conformant) | b3bb6ff3 | 2026-06-13 |
| A5 registry reconcile (edit-only part) | done | diagnostics, meta_language | cad47d06 | 2026-06-13 |

> **A5 edit-only done; existence/severity questions carried to Track C.** Done here: (1) meta_language.md `UnknownColumn`→`UndeclaredColumn` (same code under two names; 4 refs); (2) diagnostics.md split the "Incremental & cumulative" catalogue group — the ten `Cumulative*` codes now sit under a new "### Cumulative aggregate" group owned by `cumulative_aggregate.md`; `IncrementalNotBatchSafe` stays under "### Incremental" (owned by incremental_models.md; timeseries.md dropped — it owns no code here). **Carried to Track C** (not resolved): `ColumnTypeUnresolved` existence/status (D-07); `IncrementalNotBatchSafe` *trigger semantics* (D-11, ↔R2); planner-validation-code owner (D-12); `UndeclaredColumn`/`AmbiguousColumn` lsp-vs-catalogue severity (D-10).

> **A4 scope grew on sweep.** The review *sampled* template conformance; a full sweep found the gap was corpus-wide, not a handful of specs. User decided (2026-06-13): **full boilerplate sweep** + standardise label **to `**What this is.**`**. See the conformance table in Appendix A4 below. The References flat-vs-nested `### Code`/`### Tests` question (17 specs) was **left untouched** — it is a `design-decision-needed` (Track C, decision D-13).

> **Carried to Track C (decisions surfaced during sweeps):**
> - `DuplicateFunctionDefinition` scope — directory vs project vs workspace (functions.md name-uniqueness finding); A3 left it scope-neutral. → Theme 6.
> - A4 References format: flat-bullet vs nested `### Code`/`### Tests` across 17 specs is a `design-decision-needed`. → Theme 2.
> - A5 code-existence questions (does `ColumnTypeUnresolved` / `UnknownSmeltPath` fire?) → Theme 2.

### Track C — decision document
| Step | Status | Artifact | Date |
|------|--------|----------|------|
| C-prep generate markup doc | done | `docs/research/20260613-spec-remediation-decisions.md` (58 decisions D-01–D-58 + Appendix A/B) | 2026-06-13 |
| C markup returned by user | pending — **awaiting user** | | |

### Track B — per-spec no-brainer batches
| Spec | Status | Findings closed | Commit | Date |
|------|--------|-----------------|--------|------|
| **Appendix-A determinate batch** (veto-only) | done | C16, C17, C26 + 11 Maj/Min across 11 specs | 5eb056f3 | 2026-06-13 |
| lsp.md (general per-spec batch) | pending | | | |
| diagnostics.md (general per-spec batch) | pending | | | |
| meta_language.md (general per-spec batch) | pending | | | |
| *(remaining specs added as worked)* | | | | |

> **Appendix-A determinate fixes done (one commit).** The decision-doc Appendix-A veto-only list, applied as-worded across 11 specs (last_reviewed bumped on each): **C16** schema_evolution DECIMAL widening (`s2≥s` ∧ `(p2−s2)≥(p−s)`); **C17** scoping `FragmentKindMismatch` direction reversed (fires when fragment kind is *higher* than the splice point admits); **C26** gradual_typing nullability-in-signatures → points to types.md §11; types `NOW()`/`CURRENT_TIMESTAMP` non-nullable origin added to §11; types decimal integer-lifting trigger ("≥1 operand already Decimal-family"); cli seed step → seed lifecycle/`Backend::load_table` (was `read_csv_auto`); cli + data_catalog `explain` enums gain `cumulative_aggregate`; data_catalog Constraint 3 → "deterministic *key ordering*", `generated_at` flagged non-deterministic; cumulative `--start/--end` → `--event-time-start/--end`; cumulative idempotency parenthetical (idempotent combiners only: MIN/MAX/BOOL_*/BIT_AND/BIT_OR; SUM/COUNT/BIT_XOR not); incremental partition-column projection rule de-duped → links timeseries.md rule 1 as owner; expansion `CteShadowsCallerCte` named (dropped `CteCycle` hedge); seeds invariant 1 reworded; sources Constraint 6 softened + Constraint 7 drops "sources namespace" framing. *(Already landed earlier: meta_language `UnknownColumn`→`UndeclaredColumn` in A5; template conformance in A4.)* Items tagged `↔R1/R2` in Appendix A are parentheticals only — the deeper rewrites still belong to R1/R2.

### Track C — decision execution (by theme)
| Theme | Status | Findings closed | Commit | Date |
|-------|--------|-----------------|--------|------|
| 1 architecture resolution/naming | done | D-01..D-06 (C1,C2,C22 + schema-default + scan-universe + stem-rule) | e125ce73 | 2026-06-13 |
| 2 diagnostics existence/severity | done | D-07,08,09,10,12,14 (D-11→R2; D-13 = 8f05806b) | 277538a9 | 2026-06-13 |
| 3 meta-language reflection/precedence | done | D-15..D-21 (D-16,D-24 = user B) | c7773476 | 2026-06-13 |
| 4 python↔meta | done | D-22..D-27 | c7773476 | 2026-06-13 |
| 5 type system | done | D-28,D-29 | 4c1ee530 | 2026-06-13 |
| 6 project surface/config | done | D-30..D-35 (D-35 = user B) | 1426d801 | 2026-06-13 |
| 7 cli & selection | done | D-36..D-41 + D-01 scope reconcile | bab08fb0 | 2026-06-13 |
| 8 per-spec smaller calls | done | D-42..D-58 (D-44 = user B; D-45 = user model; D-51→R2) | 8495cd6f | 2026-06-13 |

**Diagnostics catalogue adds/reconciles** (cross-theme, commit `e952566b`): `HofNamedArgument` (D-19), `UnknownTestInput` + Testing group (D-43), `DuplicateFunctionDefinition` directory-scoped (D-30), `FrontmatterParseError`→Error (D-31). `DuplicateEmittedName` added in theme 1 (D-02).

### Spawned — rewrites & code waves (own plans/spec cycles)
| Item | Substantive home | Closes | Status |
|------|------------------|--------|--------|
| cumulative_aggregate rewrite | `/smelt:spec cumulative_aggregate` → R1 | C3,C4,C25 + cluster | pending |
| incremental_models rewrite | `/smelt:spec incremental_models` → R2 | C7,C8,C9,C10 + cluster | pending |
| D-diag code wave | `docs/plans/<date>-diag-rollout.md` | impl-impact diag/lsp | pending |
| D-resolve code wave | own plan | C1,C2 resolver | pending |
| D-types code wave | own plan | C16,C17 + nullability | pending |
| D-incr code wave | own plan (after R1/R2) | incremental/cumulative | pending |
| D-cli code wave | own plan | cli selectors/exit codes | pending |
| D-misc code wave | own plan | fn-schema, catalog, datagen | pending |
| ROADMAP: 3 deferred review passes | `docs/ROADMAP.md` | completeness-critic | pending |

---

## Critical files

- `docs/research/20260612-spec-review.md` — the finding source (read by line-range per session, never whole).
- `docs/specs/*.md` — the 30 specs being edited. `architecture.md` is `stable` — changes there (C1/C2/C22, schema default) need extra care.
- `docs/research/20260613-spec-remediation-decisions.md` — **new**, the markup decision doc (Session 2).
- `docs/specs/SPEC_TEMPLATE.md` — conformance oracle for Track A4.
- `docs/ROADMAP.md` — deferred review passes (final session).
- `.claude/commands/smelt/{spec,plan,validate}.md` — the rewrite/rollout machinery for R1/R2 and Track D.
- Reuse the prior `docs/plans/20260504-spec-review-followup.md` format for any spawned wave plan.

---

## Token-efficiency rules (per CLAUDE.md)

- Each session reads only this plan + target spec(s) + the cited review-doc **line range** (`Read` with `offset/limit`), never the full review.
- Use `rg` for the sweep verification greps; never `grep -r` (hits `docs-site/site/`).
- The decision document (Session 2) replaces ~68 interactive `AskUserQuestion` rounds with one offline markup pass — the single biggest LLM-call saving.
- Track A and Track B commits are mechanical; batch them, don't re-litigate `_Fix:_` lines the review already verified.

---

## Verification

- **Per sweep (A1–A3):** the `rg` assertion in each sweep returns empty / expected.
- **Track A4:** the conformance table shows every spec with scope-callout + Out-of-scope + Spec-first + Timeless-oracle blockquotes; `/smelt:validate` finds no `Phase [A-Z0-9]` leakage in spec bodies.
- **Track B/C:** for each edited spec, `last_reviewed` bumped; re-grep the closed finding's symptom to confirm the contradiction is gone (e.g. after C7, `rg 'run window' docs/specs/incremental_models.md` shows only the reconciled usage).
- **No cross-spec dangling refs:** `rg` each cross-referenced §heading exists in its target spec (the review's biggest defect class was dangling references).
- **Rewrites (R1/R2):** `/smelt:validate cumulative_aggregate` and `/smelt:validate incremental_models` produce clean drift reports against the new spec text (will list code drift → that's Track D input, expected).
- **Code waves (D):** each wave's spawned plan carries its own TDD tests + `cargo test` gate; `/smelt:validate` for the wave's specs reports zero drift at close.
- **Close-out:** Progress table fully `done`; ROADMAP carries the 3 deferred passes; PR opened to `main`.

---

## Notes on what is NOT in scope here

- No code edits in this plan (Track D is spawned). The user's directive: group code rollout by subsystem to avoid validate/plan/implement churn touching the same crate per-spec.
- The 3 completeness-critic review passes are *new review work*, not remediation — deferred to ROADMAP.
- The 3 refuted findings (appendix of the review) are intentionally not actioned.

---

## Appendix A4 — SPEC_TEMPLATE conformance table (post-sweep)

Corpus-wide sweep of all 29 specs against `SPEC_TEMPLATE.md`, replacing the review's ad-hoc per-spec "missing scope callout" findings. State **after** the A4 commit. Required header blockquotes: scope callout (`**What this is.**`, naming out-of-scope adjacent owners), Spec-first, Timeless-oracle.

| Spec | Callout | Spec-first | Timeless | A4 action taken |
|------|:------:|:----------:|:--------:|-----------------|
| architecture (`stable`) | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes (no prose touched) |
| cli | ✓ | ✓ | ✓ | +2 blockquotes |
| cumulative_aggregate | ✓ | ✓ | ✓ | label only (blockquotes pre-existing) |
| data_catalog | ✓ | ✓ | ✓ | +2 blockquotes |
| datagen | ✓ | ✓ | ✓ | +out-of-scope (seeds/sources/testing); +2 blockquotes |
| diagnostics | ✓ | ✓ | ✓ | **added** catalogue/index callout (was missing); +2 blockquotes |
| expansion | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes (2nd callout para kept) |
| function_schema_inference | ✓ | ✓ | ✓ | already conformant — untouched |
| functions | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes |
| gradual_typing | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes |
| incremental_models | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes (before Status) |
| lsp | ✓ | ✓ | ✓ | dropped "performance contract"; +out-of-scope (diagnostics/architecture); +2 blockquotes |
| meta_config_loading | ✓ | ✓ | ✓ | `Map<K,V>`→`Map<Text,S>`; +Timeless |
| meta_language | ✓ | ✓ | ✓ | +Timeless |
| model_selection | ✓ | ✓ | ✓ | +out-of-scope (cli/models/meta_language); +2 blockquotes |
| models | ✓ | ✓ | ✓ | +Timeless |
| output_fingerprint | ✓ | ✓ | ✓ | already conformant — untouched |
| planner_integration | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes; References "Phase 42"→"lowering helpers" |
| python_models | ✓ | ✓ | ✓ | +2 blockquotes |
| run_state | ✓ | ✓ | ✓ | already conformant — untouched |
| schema_evolution | ✓ | ✓ | ✓ | +out-of-scope (run_state/virtual_environments/output_fingerprint); +2 blockquotes |
| scoping | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes |
| seeds | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes; References "(as of Phase 4)" removed |
| smelt_yml | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes |
| sources | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes |
| testing | ✓ | ✓ | ✓ | +2 blockquotes (before "Naming history." blockquote) |
| timeseries | ✓ | ✓ | ✓ | label only (blockquotes pre-existing) |
| types | ✓ | ✓ | ✓ | label→What-this-is; +2 blockquotes |
| virtual_environments | ✓ | ✓ | ✓ | already conformant — untouched |

**Result: 29/29 specs carry all three required header blockquotes.** Label standardised to `**What this is.**` corpus-wide (was 16/14 split). Two timeless-oracle References-section Phase-vocab cases fixed. Frontmatter and `last_reviewed` deliberately not bumped (mechanical sweep). Body §Semantics/§Design/§Constraints structural gaps (e.g. diagnostics.md has no §Semantics/§Design) are **not** addressed here — they are content work, tracked separately.

**Deferred to Track C (not done in A4):**
- **References format** — 17 specs use nested `### Code`/`### Tests` headings vs the template's flat bullets. `design-decision-needed` → decision **D-13** (Theme 2). Untouched.
- **DuplicateAddress ownership** (scoping.md vs architecture.md) — ownership call, not mechanical. → Track C.
- **List&lt;Unknown&gt; widening discipline** in gradual_typing scope callout (review Minor) — "add to callout vs move section" is a judgment call. → Track C.
