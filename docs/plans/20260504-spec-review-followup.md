# Plan: spec review 2026-05-03 follow-up

**Date:** 2026-05-04
**Source review:** [`docs/spec-review-2026-05-03.md`](../spec-review-2026-05-03.md)
**Tracking branch:** `docs/spec-review-2026-05-03` (this worktree)
**Docs:** docs-only — no crate / example code changes in this plan
**Scope:** drive the 22 specs in `docs/specs/` to internal consistency by closing the findings in the review

---

## Context

The 2026-05-03 multi-reviewer audit (`docs/spec-review-2026-05-03.md`) found that the spec set is mid-migration: `architecture.md` was reworked on 2026-05-01 to introduce universal `smelt.<path>` addressing, models-as-functions, and a unified `paths:` config — but ~6 feature specs still describe the previous world, one config spec (`project_config.md`) duplicates `smelt_yml.md` with an incompatible schema, and several "future spec" pointers are stale.

This plan executes the cleanup. The first eight phases are **mechanical** — the review already names the source of truth for each disagreement and we follow it. The later phases are **placeholders** — they create new system-level specs (`diagnostics.md`, `run_state.md`, `multi_backend.md`, `planner_api.md`) or commit to cross-cutting design decisions, and need discussion before execution.

The leverage hierarchy from `CLAUDE.md` (1× into code, 100× into plans, 1000× into next round of specs) makes this a high-payoff plan — every fix here removes a class of downstream drift before the next implementation plan cites these specs.

## Scope

### In scope

- All 22 specs in `docs/specs/` (and `SPEC_TEMPLATE.md`).
- `last_reviewed` frontmatter bumps on every touched spec.
- Cross-references between specs (anchored where possible).

### Out of scope (for this plan)

- Implementation drift (the job of `/smelt:validate`, run after this plan lands).
- `docs-site/` user-doc audit — separate plan after Phase 1–3 land. Each phase here notes any user-doc impact but does not edit `docs-site/` unless a spec change makes a docs-site claim factually wrong.
- `examples/timeseries/` model→model reference fixture — recommended by the review but is example-code work, deferred to a separate plan.
- Implementing any new behaviour the new specs imply (e.g. a real `diagnostics.md` registry in code).

## How to use this plan

Each obvious phase is a single docs-only commit. Run them in order — Phase 1 unblocks Phases 2–3, and Phases 2–3 unblock everything else. Within a phase, the review's "Suggested fix" lines are the authoritative checklist.

There is no `/smelt:implement` TDD loop here (no code, no tests). The review **is** the oracle: each phase cites the review section it closes, and the review's per-finding "Suggested fix" is the acceptance criterion.

After each phase: `git commit` with the phase's commit line, push to the tracking branch so the user sees progress on GitHub.

The discussion phases (9–16) carry the questions that need answers before they can be executed. Mark them `pending` until aligned, then convert each into its own plan or absorb back here.

---

## Progress tracking

| Phase | Status   | Findings closed                     | Commit | Date |
|-------|----------|-------------------------------------|--------|------|
| 1     | pending  | H1 (delete), Mi1 partial            |        |      |
| 2     | pending  | M8, M6, partial H1                  |        |      |
| 3     | pending  | H2, N1, N2, Mi5                     |        |      |
| 4     | pending  | M1, M2                              |        |      |
| 5     | pending  | H3                                  |        |      |
| 6     | pending  | M3                                  |        |      |
| 7     | pending  | M4                                  |        |      |
| 8     | pending  | M5, M7, M16, Mi3, Mi7, Mi1, Mi2, N3 |        |      |
| 9     | discuss  | C1                                  |        |      |
| 10    | discuss  | H6                                  |        |      |
| 11    | discuss  | H7                                  |        |      |
| 12    | discuss  | H8                                  |        |      |
| 13    | discuss  | H4                                  |        |      |
| 14    | discuss  | H5, M9, M10, M11, M13, M15          |        |      |
| 15    | discuss  | M12                                 |        |      |
| 16    | discuss  | M14, Mi4, Mi6, Mi10–Mi19, Ergonomics |        |      |

---

# Part A — Obvious phases (executable now)

These follow the review's Suggested-fix lines directly. No new design space is opened.

### Phase 1: Delete `project_config.md`, salvage cross-engine paragraph

**Closes:** H1 (the contradictory `smelt.yml` specs).

**Source-of-truth call (per review):** `smelt_yml.md` matches the implementation, examples, and docs-site. Delete `project_config.md` outright; salvage only its cross-engine Parquet-exchange paragraph.

**Steps.**
1. Lift the cross-engine Parquet-exchange paragraph from `project_config.md` §"Cross-engine data exchange" into `architecture.md` §"Backend trait surface" (or, if Phase 12 lands first, into `multi_backend.md`). Keep wording verbatim where possible — Phase 12 may re-author it.
2. Delete `docs/specs/project_config.md`.
3. Sweep cross-references: any spec that cites `project_config.md` now cites `smelt_yml.md`. Likely sites: `cli.md`, `models.md`, `seeds.md`, `architecture.md`.
4. Bump `last_reviewed: 2026-05-04` on `architecture.md` and `smelt_yml.md`.

**Acceptance.** `rg -n "project_config" docs/specs/` returns zero results. The cross-engine paragraph is in `architecture.md`.

**Commit.** `spec: delete project_config.md, salvage cross-engine paragraph (review H1)`

---

### Phase 2: Complete the unified `paths:` migration

**Closes:** M8 (models.md/python_models.md/testing.md describe `model_paths`), M6 (`cli.md` references `seed_paths` and aggregate `sources.yml`), partial H1.

**Source-of-truth call (per review):** `paths:` is the canonical key. `model_paths`/`seed_paths` are retired everywhere they appear in specs.

**Steps.**
1. `models.md`: replace every `model_paths` reference with `paths:`. Sites: §"File format" (line ~18), §"Model name derivation" / "discovery" (line ~140).
2. `python_models.md`: same sweep — `paths:` for the discovery section.
3. `testing.md`: replace `model_paths` references in §"File discovery"-style sections.
4. `cli.md`: replace `seed_paths` and aggregate `sources.yml` in `smelt build` lifecycle (§"`smelt build` lifecycle"). Per `seeds.md` Known Divergences and `sources.md` Constraints §6, the aggregate `sources.yml` is retired — use per-entity source `.yml` files.
5. `lsp.md`: replace every `sources.yml` (singular file) reference with per-entity source `.yml` (M7 — see Phase 3 since these often co-locate with addressing changes; if Phase 3 already swept lsp.md, only verify here).
6. Bump `last_reviewed` on each touched spec.

**Acceptance.** `rg -n "model_paths|seed_paths" docs/specs/` returns zero results. `rg -n "sources\.yml" docs/specs/` returns either zero results or only references to per-entity layout.

**Commit.** `spec: complete unified paths: migration in feature specs (review M6, M7, M8)`

---

### Phase 3: Complete `smelt.<path>` addressing migration

**Closes:** H2 (legacy `smelt.models.<name>` / `smelt.sources.<schema>.<table>` in 6 specs), N1 (mixed addressing within `incremental_models.md`), N2 (broken `smelt-yml.md` hyphen typo), Mi5 (lsp.md's `MalformedSource`/`SourceTypeError` not in `sources.md`).

**Source-of-truth call (per review):** `smelt.<path>` is universal per `architecture.md` §"Resolution" and the 2026-05-01 design rework. Every kind-prefixed legacy form (`smelt.models.<name>`, `smelt.sources.<schema>.<table>`, `smelt.fn.<path>`) is retired.

**Steps.**
1. **Spec sweep.** Per the review's H2 file list:
   - `models.md` §"Reference syntax" — replace legacy form, add the parameterised-call surface (`smelt.<path>(filter => ...)`) to match `architecture.md` §"Models as functions". Closes Mi4.
   - `lsp.md` — every diagnostic name, every goto-definition row, every completion trigger. Rename `UndefinedModelRef` → `UnknownSmeltPath` and `UndefinedSource` → `UnknownSmeltPath` (single code, kind-aware on the resolved entity, mirroring `functions.md`'s `UnknownSmeltFn`). If it makes sense to keep two codes for kind-specific user messages, document the split here.
   - `python_models.md` — examples and §"Model name derivation".
   - `testing.md` — `inputs:` keys, mock-substitution rules, `--select` examples.
   - `model_selection.md` §"Graph traversal" — drop the kind-prefix language.
   - `data_catalog.md` — diagnostic surface inherits from `lsp.md`; verify and update.
2. **Internal consistency in `incremental_models.md`.** The spec mixes `smelt.<path>` (lines 95, 144) and `smelt.models.orders` (line 33). Use `smelt.<path>` throughout. Closes N1.
3. **Typo.** `incremental_models.md` line 38 references `smelt-yml.md` (hyphen); the file is `smelt_yml.md`. Closes N2.
4. **`sources.md` diagnostic codes.** Move `MalformedSource` and `SourceTypeError` from `lsp.md` (or co-locate) so `sources.md` Surface lists them. Phase 10's `diagnostics.md` will later make this rule formal; for now, the rule is "diagnostic codes live with the feature that owns them, with `lsp.md` linking back."
5. **Anchor in `architecture.md`.** Add a one-line note to `architecture.md` §"Resolution" pinning the migration completion date (2026-05-04) so future reviewers see the cutover landed.
6. Bump `last_reviewed` on every touched spec.

**Acceptance.**
- `rg -n "smelt\.models\." docs/specs/` returns zero hits (or only legitimate "the `smelt.<path>` form replaces the legacy `smelt.models.<name>` form" historical-call-out lines).
- `rg -n "smelt\.sources\." docs/specs/` only returns hits where `smelt.sources.<segments>` is a literal `smelt.<path>` whose first segment happens to be the directory name `sources` (legitimate).
- `rg -n "UndefinedModelRef|UndefinedSource" docs/specs/` returns zero hits.

**Commit.** `spec: complete smelt.<path> addressing migration in feature specs (review H2)`

---

### Phase 4: Drop stale "future spec" pointers

**Closes:** M1 (`expansion.md` referenced as "(when written)" in 5 places), M2 (`tests.md` referenced as future in 4 places, but `testing.md` exists).

**Source-of-truth call (per review):** Both `expansion.md` and `testing.md` exist and are substantive. Drop the "(when authored)" / "(planned)" / "future `tests.md`" markers and rewrite as plain references.

**Steps.**
1. **`expansion.md` → drop "(when written)" / "(planned)".** Sites:
   - `gradual_typing.md` line 135 ("when authored") and line 212 ("*(planned)*").
   - `planner_integration.md` line 121 ("when written") and line 209 ("when written").
   - `scoping.md` line 149 ("when authored") and line 169 ("(when authored)").
2. **`tests.md` → `testing.md`.** Sites:
   - `architecture.md` Known Divergences line 324: "future `tests.md`" → "`testing.md`".
   - `functions.md` line 21: "future `tests.md` spec" → "`testing.md`".
   - `seeds.md` lines 148, 167: "tests spec" / "`tests.md` exists" → cite `testing.md`.
   - `sources.md` line 102: "future `tests.md`" → `testing.md`.
3. Add a one-line note to `testing.md` Surface acknowledging it is the spec previously called `tests.md` in cross-references, so a Ctrl-F by a returning reader still resolves.
4. Bump `last_reviewed` on each touched spec.

**Acceptance.** `rg -n "when (written|authored)|tests\.md" docs/specs/` returns only the explicit `testing.md` form (no stale future-pointer markers).

**Commit.** `spec: drop stale future-spec pointers (review M1, M2)`

---

### Phase 5: Resolve the test-declaration split

**Closes:** H3 (`smelt.test` declaration described two ways across three specs).

**Source-of-truth call (per review):** Keep `materialization: test` (matches the implementation per `testing.md` References → `crates/smelt-core/src/metadata.rs::TestConfig`). Drop `smelt.test` as a top-level declaration kind.

**Caveat.** If we discover at execution time that the parser already accepts `smelt.test <name>` declarations and code depends on them, escalate to the discussion phase before deleting the spec text. The review's recommendation is conditional on `materialization: test` being the implemented form.

**Steps.**
1. `architecture.md` Known Divergences (line ~324): drop the `smelt.test`-as-declaration entry; the rewritten line should now say "tests are specified via `materialization: test`; see `testing.md`."
2. `architecture.md` §"Bare-model naming" / file structure: remove `smelt.test <name>` from the list of top-level declaration kinds (alongside `smelt.define` / `smelt.extern`).
3. `functions.md` §"File structure" item 4: remove the `smelt.test` line.
4. `models.md` Materialization-modes table: keep the existing `test` materialization row; ensure cross-link to `testing.md` is anchored.
5. `testing.md`: no schema change needed — already documents `materialization: test`. Add a one-line Design rationale paragraph noting that an alternative `smelt.test <name>` declaration shape was considered and rejected (point at `architecture.md` Design discipline) — this discharges `feedback_specs_include_design.md` (memory note: specs must record rejected alternatives).
6. Bump `last_reviewed`.

**Acceptance.** `rg -n "smelt\.test" docs/specs/` returns only references inside `testing.md` and the `models.md` materialization row, never as a top-level declaration kind.

**Commit.** `spec: pin test-declaration shape to materialization: test (review H3)`

---

### Phase 6: Crate-name and reference fixes in `incremental_models.md`

**Closes:** M3 (`smelt-optimizer` crate cited but does not exist; the crate is `smelt-planner`).

**Steps.**
1. `incremental_models.md` Semantics §"Batch safety classification" and References block: replace every `smelt-optimizer` with `smelt-planner`. Specific lines per the review: 167–168, 175.
2. Verify the cited paths exist: `crates/smelt-planner/src/rules/incremental.rs`, `crates/smelt-planner/src/types.rs`. (Read-only check; do not modify code.)
3. Bump `last_reviewed`.

**Acceptance.** `rg -n "smelt-optimizer" docs/specs/` returns zero hits.

**Commit.** `spec: fix smelt-optimizer references to smelt-planner in incremental_models (review M3)`

---

### Phase 7: Add missing `## Design` sections

**Closes:** M4 (`incremental_models.md` and `types.md` lack a `## Design` section).

**Source-of-truth call (per review and `feedback_specs_include_design.md`):** Every spec must have a `## Design` section per `SPEC_TEMPLATE.md`; design rationale is a normative requirement, not a stylistic choice.

**Steps.**
1. **`incremental_models.md` `## Design`.** Extract from currently-scattered notes the rationale for: (a) "no Jinja, logical SQL is pure"; (b) DELETE+INSERT over partition columns rather than MERGE; (c) "smelt does not own state" — the backend is the watermark store; (d) the BoundedSafe / FullyBatchSafe / NotBatchSafe trichotomy. Each paragraph should also name the rejected alternative (e.g. "MERGE was rejected for v1 because…").
2. **`types.md` `## Design`.** Extract: (a) strict-by-default doctrine (already settled in §"Strict-by-default doctrine" — promote to Design); (b) single-vocabulary `DataType` enum across backends; (c) no implicit coercion; (d) engine-alias normalisation as a separate concern from inference. Each paragraph names the rejected alternative.
3. Section position: after Semantics, before Constraints, per `SPEC_TEMPLATE.md`.
4. Bump `last_reviewed`.

**Acceptance.** `rg -n "^## Design" docs/specs/incremental_models.md docs/specs/types.md` returns exactly one match per file.

**Commit.** `spec: add ## Design sections to incremental_models and types (review M4)`

---

### Phase 8: Mid-priority mechanical cleanup

**Closes:** M5 (unknown-key handling inconsistency), M16 (README/CLAUDE.md differentiator-list mismatch), Mi1 (`stable` undefined), Mi2 (`last_reviewed` lag), Mi3 (broken `sources.md` ↔ `seeds.md` cross-references), Mi7 (tag case-sensitivity location), N3 (References-block shape variance).

This phase is a grab-bag of low-risk fixes. Each step is independent.

**Steps.**

1. **Unknown-key doctrine (M5).** Add a single sentence to `architecture.md` §"Constraints & Invariants" stating the doctrine: *"User-authored content (model frontmatter, type annotations) is strict; project-level config (`smelt.yml`) is lenient with warnings."* Then have `models.md`, `smelt_yml.md`, and `functions.md` reference the doctrine instead of restating it. (The contradictory `project_config.md` was already deleted in Phase 1.)
2. **README ↔ CLAUDE.md differentiators (M16).** Reconcile the two lists. Pick one canonical set (the `CLAUDE.md` six-item version — it is more granular and matches the spec set) and mirror it in `README.md`. Note: `README.md` is at the repo root, not in `docs/specs/`, so this step touches a non-spec file — flag in the commit message.
3. **`SPEC_TEMPLATE.md` `status:` values (Mi1).** Define the allowed values: `experimental`, `stable`, `deprecated` (or whatever the project wants). Promote `architecture.md`'s `stable` into a documented promise. If we cannot say what `stable` promises today, demote `architecture.md` to `experimental` and remove the `stable` enum value until Phase 14 (journey integrity) returns to it.
4. **`last_reviewed` audit (Mi2).** For every spec with a `last_reviewed` date earlier than 2026-05-01 — currently `incremental_models.md` (2026-04-27) and `types.md` (2026-04-28) — re-read the spec under the addressing-scheme rules (Phase 3 will have already done the substantive sweep) and bump the date. The seven `2026-04-29` specs in the function family also need this.
5. **`sources.md` ↔ `seeds.md` cross-references (Mi3).** `seeds.md` cites `sources.md` §"Source YAML shape" — verify the heading exists. `sources.md` cites `seeds.md` §"Sidecar / source YAML shape" — that heading does not exist. Fix the back-reference: the canonical home is `sources.md` §"Source YAML shape"; `seeds.md` cross-references it.
6. **Tag case-sensitivity (Mi7).** Move the case-sensitivity rule from `model_selection.md` Constraints §5 into `models.md` Semantics §"Tag merging" so it co-locates with the merge rule. `model_selection.md` cross-references it.
7. **References-block shape (N3).** Pick one form (recommend flat bullets per the most common pattern) and update `SPEC_TEMPLATE.md` to say so. No spec rewrite needed in this phase — the inconsistency is grandfathered.

**Acceptance.** Each of the seven steps lands; per-step verification:
- step 1: `rg -n "deny_unknown_fields|hard error.*key|silent.*key" docs/specs/` shows the rule appears once authoritatively (in `architecture.md`) and is referenced elsewhere.
- step 2: differentiator lists in `README.md` and `CLAUDE.md` are syntactically identical (same N items, same ordering, same titles).
- step 3: `SPEC_TEMPLATE.md` defines `status:` values; `architecture.md`'s frontmatter is consistent with it.
- step 4: `rg -n "last_reviewed: 2026-04" docs/specs/` returns zero hits.
- step 5: both cross-references resolve.
- step 6: `rg -n "case-sensitive|case-insensitive" docs/specs/` shows the rule lives in `models.md`.
- step 7: `SPEC_TEMPLATE.md` documents the chosen References-block form.

**Commit.** `spec: cross-spec doctrine, dates, and cross-reference cleanup (review M5, M16, Mi1–Mi3, Mi7, N3)`

---

# Part B — Discussion phases (need decisions before execution)

These open new design space. Each carries the questions that must be answered before the phase becomes executable. They are listed in roughly the order recommended by the review's PR sequence.

### Phase 9: Multi-model file format (placeholder)

**Closes:** C1 (Critical: `architecture.md` says per-declaration YAML frontmatter with `name:`; `models.md` / `python_models.md` / `testing.md` say `--- name: <name> ---` section delimiters).

**Why deferred.** The review states "a parser cannot satisfy both rules" — these are two distinct designs. Before deleting one, we need to confirm which the implementation actually parses today (read `crates/smelt-parser/src/parser.rs` and recent fixtures) and pick the spec wording to match. Either outcome forces a non-trivial rewrite of one or more specs.

**Open questions for discussion.**
1. Which form does the parser implement today?
2. If `--- name: X ---` delimiter form is canonical (the wider-cited form), what does `architecture.md` §"Bare-model naming" become?
3. How does the chosen form interact with `models.md` Known Divergences §"Duplicate model names undefined"?

**Likely commit.** `spec: pin multi-model file format to <chosen form> (review C1)`

---

### Phase 10: New system spec — `diagnostics.md` (placeholder)

**Closes:** H6 (no central diagnostic-code catalogue; codes scattered across 6 specs).

**Why deferred.** This is a new spec, not a sweep. Its shape is a design decision: registry table format, ownership rule (one spec per code), severity tiers, stability tiers (stable / experimental / internal), suppression mechanism. The review proposes the rule "each code is *owned* by exactly one spec; others may reference but must not redefine the trigger" — that needs to be ratified.

**Open questions for discussion.**
1. Index spec (linking to per-feature catalogues) or canonical home (codes move into `diagnostics.md` itself)?
2. Stability tiers: do we promise stable codes today, or is everything experimental for now?
3. Severity tiers: does the spec use Error / Warning / Hint / Info, matching LSP, or a smaller set?
4. Suppression: do we need a `# smelt-allow: CodeName` mechanism in this spec or defer to a later one?

**Inputs to read before drafting.** All six current code lists: `functions.md`, `gradual_typing.md`, `scoping.md`, `lsp.md`, `types.md`, `planner_integration.md`. Plus `incremental_models.md` and `schema_evolution.md`, which the review notes also mention codes informally.

**Likely commit.** `spec: add diagnostics.md (review H6)`

---

### Phase 11: New system spec — `run_state.md` (placeholder)

**Closes:** H7 (no spec covers run state, build orchestration, or observability).

**Why deferred.** New spec. Scope is wide and design space is open: manifest format, `IntervalStore`, `FileStore`, `.smelt/` layout, run IDs, parallelism, failure recovery, log/output format. Review notes this is "the dbt `manifest.json` story" and says it's the area least ready to back the "production ready" claim. Needs an explicit scope decision before drafting.

**Open questions for discussion.**
1. Single spec covering all of `.smelt/` lifecycle, or split between `run_state.md` (run records, manifest) and `build_lifecycle.md` (orchestration)?
2. What's stable today vs aspirational? `RunManifest`, `IntervalStore`, `FileStore` are referenced in `architecture.md` Crate table as code constructs — is the on-disk format stable or pre-v1?
3. Does this spec own the `smelt status` and `smelt history` Surface, with `cli.md` cross-referencing? Or vice versa?
4. Log format / observability — in scope or out?

**Inputs to read.** `architecture.md` Crate table; `cli.md` `smelt status` / `smelt history`; `schema_evolution.md` `.smelt/schemas/` references; `incremental_models.md` §"State ownership".

**Likely commit.** `spec: add run_state.md (review H7)`

---

### Phase 12: Multi-backend execution model (placeholder)

**Closes:** H8 (no spec covers multi-backend execution end-to-end), partial M14 (Decimal arithmetic interaction with backends).

**Why deferred.** Two paths in the review:
- (a) expand `architecture.md` §"Backend trait surface" into a full §"Multi-backend execution model" — preferred, cohabits with the existing trait surface.
- (b) create `multi_backend.md` — preferred if the section grows large enough to warrant its own file.

The Phase 1 salvage of the cross-engine Parquet paragraph already lands content here; Phase 12 is the full design.

**Open questions for discussion.**
1. (a) vs (b) — expand `architecture.md` or new file?
2. What does v1 promise? Minimal v1: "DuckDB default; Spark via `target:`; cross-engine via Parquet handoff at materialised boundaries; tests on DuckDB only." Anything beyond that needs a stake in the ground.
3. Capability-negotiation: does `architecture.md` Backend trait surface grow capability flags (incremental support, MERGE support, ALTER COLUMN support)? Today this is scattered across `incremental_models.md`, `schema_evolution.md`.
4. Cross-engine reference resolution rules: when a model on backend A reads from a model on backend B, where does `read_parquet()` substitution happen? When does it not work?

**Inputs to read.** `architecture.md` §"Backend trait surface", `incremental_models.md` Known Divergences (Spark MERGE), `testing.md` Known Divergences (Spark test gap), `schema_evolution.md` capability matrix, `smelt_yml.md` Known Divergences (multi-target precedence).

**Likely commit.** `spec: add multi-backend execution model (review H8)` (or a `architecture.md` expansion commit, depending on (a)/(b)).

---

### Phase 13: Planner extensibility surface (placeholder)

**Closes:** H4 (engineer-controls-planning differentiator has no public spec surface).

**Why deferred.** Two paths in the review:
- (a) `Known Divergences` entry in `planner_integration.md` saying "user-authored planner-rule API is in scope but pre-spec."
- (b) pull `docs/planner_rule_api_design.md` into `docs/specs/planner_api.md` as a stub — preferred by the review.

(b) is preferred because it puts the differentiator on the spec map even if the API is unstable. But it requires deciding what's already pinned vs aspirational, and the existing `docs/planner_rule_api_design.md` was authored before the universal-addressing rework — needs a re-read first.

**Open questions for discussion.**
1. (a) divergence entry, or (b) stub spec?
2. If (b): what shape — trait outline, registration call, lifecycle, stability disclaimer? Or as much of `docs/planner_rule_api_design.md` as still applies?
3. Naming: `planner_api.md` per the review, or `planner_rules.md` to match `planner_integration.md`?

**Inputs to read.** `docs/planner_rule_api_design.md` (existing design doc), `planner_integration.md` (current spec), `architecture.md` §"`Transformation` and `ExecutionStep`".

**Likely commit.** `spec: add planner_api.md stub (review H4)` or `spec: divergence entry for planner extensibility (review H4)`.

---

### Phase 14: Cross-cutting journey integrity (placeholder)

**Closes:** H5 (testing × incremental × schema-evolution × multi-backend gaps), and absorbs the related M-level findings: M9 (incremental first-run / partial-failure / late-arrival), M10 (schema-evolution flags missing from `cli.md` `smelt build` table), M11 (`smelt test --select` substring asymmetry), M13 (functions inside incremental bodies — pushdown vs safety), M15 (selector edge cases).

**Why deferred.** This is partly mechanical (M10 is just a flag-table edit, M11 is a Surface-level note) and partly design (the journey-integrity matrix the review proposes — where it lives, what it lists, how it's maintained — is open).

**Open questions for discussion.**
1. "User journey integrity matrix" — in `architecture.md` Constraints, or a new `cross_cutting.md` (or both, with `architecture.md` linking)?
2. M9 (incremental first-run / partial-failure): is the Phase 11 `run_state.md` the right home for chunking and transaction-boundary semantics, or `incremental_models.md`?
3. M13 (functions inside incremental bodies): does the answer go in `incremental_models.md` §"Functions inside incremental bodies" (review's recommendation), `functions.md`, or `planner_integration.md`?
4. M15 (selector edge cases): does empty selection warn and exit 0, or error? Does `--exclude` orphaning warn or error?
5. Schema evolution × incremental: section in `schema_evolution.md`, or stub `incremental_schema_evolution.md`?

**Inputs to read.** `incremental_models.md` Known Divergences, `testing.md` Known Divergences (Spark test gap), `schema_evolution.md`, `models.md` (`incremental:` frontmatter without target restriction), `smelt_yml.md` Known Divergences (multi-target precedence), `model_selection.md` Known Divergences.

**Likely commit.** Multiple — likely split into 3 sub-commits: (a) `cli.md` flag-table fix (M10) + `testing.md` `--select` Surface note (M11), (b) `incremental_models.md` first-run / functions sections (M9, M13), (c) journey integrity matrix + edge cases (H5, M15).

---

### Phase 15: dbt migration story (placeholder)

**Closes:** M12 (no spec mentions dbt anywhere; entire adoption funnel is migrating from dbt).

**Why deferred.** Two paths in the review:
- (a) dbt-comparison section in `architecture.md`.
- (b) dedicated `migration_from_dbt.md` spec — could live in `docs/specs/` as a normative analogue table or in `docs-site/` as a guide.

A dedicated spec adds the differentiator to the spec map; a section is faster but lower-visibility. Also: this lives uncomfortably between spec-territory and user-doc territory — the review treats it as spec-shaped because it pins the *semantic* mapping (what `ref()` becomes, what `is_incremental()` becomes), but the audience is not implementers.

**Open questions for discussion.**
1. (a) section in `architecture.md` or (b) standalone spec?
2. Scope: just the analogue table (`ref` → `smelt.<path>`, `source` → `smelt.<path>`, `is_incremental()` → injected `WHERE`, etc.), or also the migration mechanics (what does an automated migration look like)?
3. If standalone: `migration_from_dbt.md` or `dbt_compatibility.md`?

**Likely commit.** `spec: dbt comparison and migration story (review M12)` (form depends on chosen path).

---

### Phase 16: Other M-level open questions and ergonomics (placeholder)

**Closes:** M14 (Decimal precision arithmetic), Mi4 (already addressed via Phase 3 Step 1), Mi6 (data_catalog "Tests" column), Mi8 (typing-quartet exemplar callout), Mi9 (Design sections lacking rejected alternatives), Mi10 (`unstable_schema:` discoverability), Mi11 (`smelt docs path` no-op), Mi12 (`smelt explain` test exclusion), Mi13 (ephemeral seed size), Mi14 (CSV strict defaults), Mi15 (`--show-plan` positional requirement), Mi16 (`smelt build --dry-run`), Mi17 (`columns:` frontmatter split), Mi18 (PASSING context-sensitivity), Mi19 (compile/runtime CSV inference divergence), and review's "Ergonomics red flags" block.

**Why deferred.** Heterogeneous: each is a small decision but most have at least one open knob. Some are nearly mechanical (Mi4, Mi6) and could be batched into Phase 8 if we want; others (M14 Decimal arithmetic, Mi17 `columns:` canonical home) are real design questions. Recommended: break into a checklist when scheduling, decide which become quick spec edits and which need their own `/smelt:spec` pass.

**Notable items.**
- **M14 Decimal arithmetic.** `types.md` Known Divergences leaves Decimal-precision arithmetic deferred. The review notes `Decimal(p,s)` arithmetic produces `Decimal(38,10)` (v1 fallback) where DuckDB native is `Decimal(19,2)`. Either finalise in `types.md` Design + Semantics or document the v1 fallback prominently in user docs.
- **Mi17 canonical `columns:` home.** Currently split across `models.md`, `schema_evolution.md`, `data_catalog.md`, `testing.md`. Pick one — likely `models.md` — and have the others reference it.
- **Mi9 Design rejection paragraphs.** `feedback_specs_include_design.md` requires Design sections to record rejected alternatives. Audit every Design section that doesn't yet, against `architecture.md` and `gradual_typing.md` as the bar.
- **Mi8 typing-quartet exemplar.** Lift up `gradual_typing.md`'s "Scope" callout pattern as the bar. Consider promoting "every spec opens with a Scope callout naming adjacent specs" into `SPEC_TEMPLATE.md`.

**Likely commits.** Multiple, one per cluster: M14, Mi9 audit, Mi17 columns home, ergonomics red flags as its own commit, etc.

---

## Verification (after all obvious phases land)

Before declaring Part A complete, run:

```sh
# 1. No legacy addressing in any spec
rg -n "smelt\.models\.|UndefinedModelRef|UndefinedSource" docs/specs/

# 2. No legacy paths config
rg -n "model_paths|seed_paths" docs/specs/

# 3. No stale future pointers
rg -n "when (written|authored)|tests\.md(?!.*testing)" docs/specs/

# 4. project_config gone
rg -n "project_config" docs/specs/

# 5. Crate name fixed
rg -n "smelt-optimizer" docs/specs/

# 6. Every spec has Design section
for f in docs/specs/*.md; do
  grep -L "^## Design" "$f" 2>/dev/null
done   # only SPEC_TEMPLATE.md should remain unmatched

# 7. last_reviewed dates current
rg -n "last_reviewed: 2026-04" docs/specs/
```

All seven should return empty (or only SPEC_TEMPLATE.md for #6).

Then:
- Re-read `docs/spec-review-2026-05-03.md` Headline-findings section. Every Critical except C1 should now be addressed. C1 is the discussion-phase entry into Part B.
- Run `/smelt:validate` on the most-touched specs (`architecture.md`, `models.md`, `testing.md`, `lsp.md`) to surface implementation drift exposed by the spec changes.

## Notes for the agent executing this plan

- **Do not edit `docs/specs/` content beyond what each phase calls for.** The review is the oracle; if the review doesn't name something, defer it.
- **Bump `last_reviewed: 2026-05-04` on every touched spec.** This is the audit signal future reviewers use.
- **Anchor cross-references where possible** (`architecture.md#resolution`) instead of bare filename — Phase 8 step 7 will codify this; meanwhile prefer anchors when adding new cross-references.
- **Commit-and-push at each phase boundary** so the user sees progress on the tracking PR.
- **No code changes in this plan.** Verification commands are read-only `rg` / `grep`. If a spec change implies a code change, note it in the phase commit message and let `/smelt:validate` surface the drift.
