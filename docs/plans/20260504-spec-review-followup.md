# Plan: spec review 2026-05-03 follow-up

**Date:** 2026-05-04
**Source review:** [`docs/spec-review-2026-05-03.md`](../spec-review-2026-05-03.md)
**Tracking branch:** `docs/spec-review-2026-05-03` (this worktree)
**Docs:** docs-only — no crate / example code changes in this plan
**Scope:** drive the 22 specs in `docs/specs/` to internal consistency by closing the findings in the review

---

## Context

The 2026-05-03 multi-reviewer audit (`docs/spec-review-2026-05-03.md`) found that the spec set is mid-migration: `architecture.md` was reworked on 2026-05-01 to introduce universal `smelt.<path>` addressing, models-as-functions, and a unified `paths:` config — but ~6 feature specs still describe the previous world, one config spec (`project_config.md`) duplicates `smelt_yml.md` with an incompatible schema, and several "future spec" pointers are stale.

This plan executes the cleanup. Phases 1–8 are mechanical sweeps following the review's "Suggested fix" lines. Phases 9, 13, 14a/b, 16a/b/c are smaller cross-spec consistency edits with the design questions answered up-front. Phase 17 closes the open work that *can't* land in this plan by anchoring it as Known Divergences inside the specs that touch it — so future plans citing those specs inherit the gaps explicitly rather than relying on a sidecar backlog.

The leverage hierarchy from `CLAUDE.md` (1× into code, 100× into plans, 1000× into next round of specs) makes this a high-payoff plan — every fix here removes a class of downstream drift before the next implementation plan cites these specs.

## Scope

### In scope

- All 22 specs in `docs/specs/` (and `SPEC_TEMPLATE.md`).
- `last_reviewed` frontmatter bumps on every touched spec.
- Cross-references between specs (anchored where possible).
- In-spec Known Divergence anchors for planned-but-unauthored specs (Phase 17).

### Out of scope (for this plan)

- Implementation drift (the job of `/smelt:validate`, run after this plan lands).
- `docs-site/` user-doc audit — separate plan after Phases 1–3 land. Each phase here notes any user-doc impact but does not edit `docs-site/` unless a spec change makes a docs-site claim factually wrong.
- `examples/timeseries/` model→model reference fixture — recommended by the review but is example-code work, deferred to a separate plan.
- Authoring the planned new specs themselves (`diagnostics.md`, `run_state.md`, multi-backend, `planner_api.md`, `migration_from_dbt.md`). Each gets a Known Divergence anchor (Phase 17) but the spec content is its own future `/smelt:spec` work.

## How to use this plan

Each phase is a single docs-only commit. Run them in order — Phase 1 unblocks Phases 2–3, Phases 2–3 unblock everything else, Phase 17 should run last because it cross-references work the other phases land. Within a phase, the review's "Suggested fix" lines (or this plan's per-phase steps) are the authoritative checklist.

There is no `/smelt:implement` TDD loop here (no code, no tests). The review **is** the oracle: each phase cites the review section it closes, and the review's per-finding "Suggested fix" plus this plan's design calls are the acceptance criteria.

After each phase: `git commit` with the phase's commit line, push to the tracking branch so the user sees progress on GitHub.

---

## Progress tracking

### Executable phases

| Phase | Status   | Findings closed                     | Commit  | Date       |
|-------|----------|-------------------------------------|---------|------------|
| 1     | done     | H1                                  | 0e76ac2 | 2026-05-04 |
| 2     | done     | M6, M8 (M7 deferred to Phase 3)     | 17e7b6f | 2026-05-04 |
| 3     | done     | H2, M7, N1, N2, Mi4, Mi5            | 7fa3db0 | 2026-05-04 |
| 4     | done     | M1, M2                              | 9d912c2 | 2026-05-04 |
| 5     | done     | H3                                  | a93b4a5 | 2026-05-04 |
| 6     | done     | M3                                  | 4ba5e51 | 2026-05-04 |
| 7     | done     | M4                                  | 483a6ce | 2026-05-04 |
| 8     | done     | M5, M16, Mi1, Mi2, Mi3, Mi7, N3     | 261cdd9 | 2026-05-04 |
| 9     | done     | C1                                  | ba277e5 | 2026-05-04 |
| 13    | done     | H4                                  | 5fa0f10 | 2026-05-04 |
| 14a   | done     | M10, M11                            |         | 2026-05-04 |
| 14b   | ready    | M9, M13                             |         |            |
| 16a   | ready    | Mi6, Mi8, Mi17                      |         |            |
| 16b   | ready    | Mi10–Mi16, Mi18, Mi19               |         |            |
| 16c   | ready    | Mi9                                 |         |            |
| 17    | ready    | H6, H7, H8, M12, M14, plus 14c      |         |            |

### Spawned to follow-up plans / specs

These are anchored as in-spec Known Divergences via Phase 17, but their substantive resolution requires its own `/smelt:spec` or `/smelt:plan` cycle. They are NOT part of this plan's executable scope.

| Origin | Substantive home | Anchor in this plan |
|--------|------------------|---------------------|
| Phase 10 — diagnostic-code catalogue | `/smelt:spec diagnostics` | Phase 17 step 1: 7 in-spec divergences + architecture.md anchor |
| Phase 11 — run state / build orchestration | research → `/smelt:spec run_state` | Phase 17 step 2: cli.md / incremental_models.md / schema_evolution.md + architecture.md anchor |
| Phase 12 — multi-backend execution | `/smelt:spec multi_backend` (or architecture.md expansion) | Phase 17 step 3: architecture.md §"Backend trait surface" |
| Phase 14c — journey integrity matrix | depends on 11 & 12 landing | Phase 17 step 4: architecture.md Constraints |
| Phase 15 — dbt migration story | `docs-site/` guide + small architecture.md pointer | Phase 17 step 5: architecture.md Known Divergences |
| Phase 16-M14 — Decimal arithmetic | `/smelt:spec types` (Decimal-precision pass) | Phase 17 step 6: types.md Known Divergences strengthen |

---

# Phases

### Phase 1: Delete `project_config.md`, salvage cross-engine paragraph

**Closes:** H1 (the contradictory `smelt.yml` specs).

**Source-of-truth call (per review):** `smelt_yml.md` matches the implementation, examples, and docs-site. Delete `project_config.md` outright; salvage only its cross-engine Parquet-exchange paragraph.

**Steps.**
1. Lift the cross-engine Parquet-exchange paragraph from `project_config.md` §"Cross-engine data exchange" into `architecture.md` §"Backend trait surface" (or, if Phase 12 lands first — which it won't in this plan; see Phase 17 — into `multi_backend.md`). Keep wording verbatim where possible.
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
1. `models.md`: replace every `model_paths` reference with `paths:`.
2. `python_models.md`: same sweep.
3. `testing.md`: replace `model_paths` references in §"File discovery"-style sections.
4. `cli.md`: replace `seed_paths` and aggregate `sources.yml` in `smelt build` lifecycle.
5. `lsp.md`: replace every `sources.yml` (singular file) reference with per-entity source `.yml`.
6. Bump `last_reviewed` on each touched spec.

**Acceptance.** `rg -n "model_paths|seed_paths" docs/specs/` returns zero results.

**Commit.** `spec: complete unified paths: migration in feature specs (review M6, M7, M8)`

---

### Phase 3: Complete `smelt.<path>` addressing migration

**Closes:** H2, N1, N2, Mi5.

**Steps.** Per the review's H2 file list — sweep `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md` for legacy `smelt.models.<name>` / `smelt.sources.<schema>.<table>` forms; align on `smelt.<path>` per `architecture.md` §"Resolution". Fix `incremental_models.md` mixed-addressing (N1) and the `smelt-yml.md` hyphen typo (N2). Move `MalformedSource` / `SourceTypeError` co-location for Mi5.

**Commit.** `spec: complete smelt.<path> addressing migration in feature specs (review H2)`

---

### Phase 4: Drop stale "future spec" pointers

**Closes:** M1, M2.

**Steps.** Drop "(when written)" / "(planned)" markers around `expansion.md` (5 sites) and `tests.md` (4 sites). Both specs exist; rewrite as plain references.

**Commit.** `spec: drop stale future-spec pointers (review M1, M2)`

---

### Phase 5: Resolve the test-declaration split

**Closes:** H3.

**Source-of-truth call:** Keep `materialization: test`. Drop `smelt.test` as a top-level declaration kind (verified against `crates/smelt-core/src/metadata.rs::TestConfig`).

**Commit.** `spec: pin test-declaration shape to materialization: test (review H3)`

---

### Phase 6: Crate-name and reference fixes in `incremental_models.md`

**Closes:** M3 (`smelt-optimizer` crate cited but does not exist; the crate is `smelt-planner`).

**Commit.** `spec: fix smelt-optimizer references to smelt-planner in incremental_models (review M3)`

---

### Phase 7: Add missing `## Design` sections

**Closes:** M4 (`incremental_models.md` and `types.md` lack a `## Design` section).

**Commit.** `spec: add ## Design sections to incremental_models and types (review M4)`

---

### Phase 8: Mid-priority mechanical cleanup

**Closes:** M5, M16, Mi1, Mi2, Mi3, Mi7, N3.

A grab-bag of low-risk fixes: unknown-key doctrine in `architecture.md`; README/CLAUDE.md differentiator alignment; `SPEC_TEMPLATE.md` `status:` enum; `last_reviewed` audit; `sources.md`/`seeds.md` cross-reference fixes; tag case-sensitivity colocation; References-block shape choice.

**Commit.** `spec: cross-spec doctrine, dates, and cross-reference cleanup (review M5, M16, Mi1–Mi3, Mi7, N3)`

---

### Phase 9: Pin multi-model file format to two-layer stack

**Closes:** C1.

**Source-of-truth call.** Implementation is two-layer: `smelt-core/src/metadata.rs` splits multi-model files on `--- name: <name> ---` *section delimiter* lines (Layer 1, "section delimiter"); `smelt-parser/src/lib.rs` then attaches bare `---` / `---` *declaration frontmatter* blocks to the immediately-following declaration within each section (Layer 2, "declaration frontmatter"). `models.md` / `python_models.md` / `testing.md` describe Layer 1 correctly. `architecture.md` §"Bare-model naming" conflates the two layers and is the spec to fix.

**Steps.**

1. **`architecture.md` §"Bare-model naming"** (line ~108): replace "must declare `name:` in its frontmatter" with "must declare itself with a `--- name: <name> ---` section delimiter (Layer 1)". Add a paragraph above or beside §"Unified frontmatter rule" introducing the two-layer stack: Layer 1 (section delimiter) splits the file into model sections; Layer 2 (declaration frontmatter) is the per-declaration YAML frontmatter that lives inside each section. Cross-link to `models.md` §"File format" for the canonical syntax.
2. **`architecture.md` resolution / file structure**: any line that says "the `name:` key in YAML frontmatter is the source of identity for multi-model files" gets corrected — identity comes from the Layer 1 delimiter; the YAML `name:` key is ignored.
3. **`models.md` §"Model name derivation"** (line 58): keep the existing "accepted but has no effect" line for the YAML `name:` key, but cross-reference architecture.md so a future reader doesn't have to chase the contradiction.
4. **Known Divergence (`models.md` line 196).** "Duplicate model names undefined" applies to **both** layers: same Layer 1 name twice within one file, AND same model name across two files. Update the divergence to say so explicitly. During execution, verify whether `crates/smelt-core/src/metadata.rs` already errors on within-file duplicates — if so, document the existing behaviour rather than declaring it undefined.
5. **`testing.md`/`python_models.md`**: no schema change; verify wording doesn't accidentally imply YAML-frontmatter-as-name.
6. Bump `last_reviewed` on each touched spec.

**Acceptance.**
- `rg -n "section delimiter|declaration frontmatter" docs/specs/architecture.md` returns ≥1 hit.
- `models.md:196` divergence covers both within-file and cross-file duplicates.

**Commit.** `spec: pin multi-model file format to two-layer stack (review C1)`

---

### Phase 13: Planner extensibility surface — divergence note

**Closes:** H4.

**Source-of-truth call.** The user-authored planner-rule API (the "engineer controls the planner" differentiator) is in scope but pre-spec. A working design exists at `docs/planner_rule_api_design.md` (210 lines, predates the 2026-05-01 universal-addressing rework — needs review before becoming normative). A stub spec is premature: putting an unstable surface on the spec map creates more rot than coverage.

**Steps.**

1. Add a Known Divergence to `planner_integration.md` §"Known Divergences / Open Questions" (after the existing 7 entries):
   > **User-authored planner-rule API — pre-spec.** Today, only built-in rules ship (the four L1 rules in `show_plan_rules()`). The `Rule` trait and `RuleContext` are reusable, but the surface for a **user-authored** rule — registration, lifecycle, stability guarantees — is not specified. The `README.md` / `CLAUDE.md` differentiator "engineer controls planning" describes intent; the working design lives at `docs/planner_rule_api_design.md` and predates the 2026-05-01 universal-addressing rework, so it needs review before becoming normative. A future `planner_api.md` spec is in scope (see `architecture.md` §"Specs not yet authored").
2. Bump `last_reviewed` on `planner_integration.md`.

**Acceptance.** `rg -n "planner_api\.md" docs/specs/planner_integration.md` returns at least one hit.

**Commit.** `spec: divergence note for planner extensibility surface (review H4)`

---

### Phase 14a: Flag-table coverage and selector asymmetry surfacing

**Closes:** M10, M11.

**Steps.**

1. **M10 — schema-evolution flags missing from `smelt build`.** `cli.md` §"`smelt build` flags" omits `--allow-column-removal` and `--allow-full-refresh`. Source-of-truth call: `schema_evolution.md` is the canonical home (matches cli.md's doctrine: "Flag enumerations are in docs-site; this spec covers behavior"). In `cli.md`, add a one-line note immediately after the `smelt build` flag table:
   > `smelt build` also accepts the schema-evolution flags `--allow-column-removal` and `--allow-full-refresh`; see `schema_evolution.md` §"Evolution flags" for semantics.
   Apply the symmetric note to `smelt run` if its flag table doesn't already cover them.

2. **M11 — `smelt test --select` substring asymmetry.** Surface the deviation where the user looks — in `testing.md` Surface, not buried in cross-spec Known Divergences. In `testing.md` §"Execution model" (or a new sibling §"Selector behaviour"):
   > **`--select` is substring-match.** Unlike `smelt run` / `smelt build` / `smelt explain`, `smelt test --select <expr>` matches `<expr>` as a plain substring against test names — the `tag:` / `path:` / `+upstream` / `downstream+` selector grammar in `model_selection.md` does **not** apply. Tracked as a divergence in `model_selection.md` Known Divergences; aligning the two is open work.
   Keep the existing Known Divergence entries in `model_selection.md` and `cli.md`.

3. Bump `last_reviewed` on `cli.md` and `testing.md`.

**Acceptance.**
- `rg -n "allow-column-removal" docs/specs/cli.md` returns at least one hit.
- `rg -n "substring|--select" docs/specs/testing.md` shows the asymmetry in Surface.

**Commit.** `spec: surface schema-evolution flags on smelt build and --select substring-match in testing (review M10, M11)`

---

### Phase 14b: Incremental first-run, partial-failure, and function-call interactions

**Closes:** M9, M13.

**Source-of-truth call (design answers).**

| Question | Answer |
|---|---|
| Chunking — `FullyBatchSafe` | Single DELETE+INSERT pair for any `[start, end)`. No chunking. |
| Chunking — `BoundedSafe(n)` | Auto-sized sub-ranges (existing 3× context, clamped 7–90 partitions rule). Each sub-range is one DELETE+INSERT pair, executed sequentially in temporal order. |
| Chunking — `PerPartitionOnly` | One partition per iteration, sequential, temporal order. Each partition is one DELETE+INSERT pair. |
| Per-chunk transaction boundary | Each chunk's DELETE+INSERT is one backend transaction. INSERT failure rolls back the chunk's DELETE. Earlier committed chunks do not roll back. |
| Failure mode | Run halts at first failed chunk, exits non-zero. Re-running the same `[start, end)` resumes correctly because every committed chunk is idempotent. |
| Late-arriving data (interim) | No automatic handling. Mitigations: trail `--event-time-end` behind real-time by the source's latency window, or run with overlapping ranges. The `data_latency:` annotation is the planned automated mechanism (already a Known Divergence). |
| Filter into function bodies | The per-model WHERE is **not** pushed into call sites (Constraint 4 stays). But transparent-function expansion (`ExpandTransparentFunctionCalls` L1 rule) happens **before** WHERE injection, so the injected filter applies to columns visible after expansion. |
| Batch-safety through function calls | Classifier walks transparent (`smelt.define`-resolved) call bodies; window frames / LAG / LEAD inside a transparent body propagate to the caller's class. Opaque calls (`smelt.extern`, built-ins without known shape) conservatively force `PerPartitionOnly` or reject — verify against `crates/smelt-planner/src/rules/incremental.rs` during execution to pin the exact behaviour. |

**Steps.**

1. **Add `incremental_models.md` Semantics §"First-run and backfill"** (after §"Execution model", before §"Batch safety classification"). Document the per-class chunking table; the per-chunk transaction boundary; the failure mode (halt-then-resume); and late-arriving-data interim guidance. Cross-link to `data_latency:` Known Divergence.

2. **Add `incremental_models.md` Semantics §"Functions inside incremental bodies"** (after §"Safety checks"). State the per-model WHERE rule (cross-link to Constraint 4) and clarify that transparent-function expansion happens before WHERE injection. State that batch-safety classification walks transparent bodies; opaque calls force `PerPartitionOnly` (or reject — verified during execution). Cross-link to `planner_integration.md` §"Optimization boundary: transparent vs black-box".

3. **Update Constraint 4** (`incremental_models.md` line 156) to add a half-sentence: *"Source-level filtering depends on temporal-dependency analysis (planned); function-body filtering happens via the L1 expand-transparent-function rule (`planner_integration.md`), not via pushdown."*

4. **Cross-references in `functions.md` and `planner_integration.md`.** One-line cross-reference each to `incremental_models.md` §"Functions inside incremental bodies" — do not restate the rule.

5. **Verify against `crates/smelt-planner/src/rules/incremental.rs`** during execution: confirm whether opaque calls force `PerPartitionOnly` or reject, and pin the spec wording to match.

6. Bump `last_reviewed` on `incremental_models.md`, `functions.md`, `planner_integration.md`.

**Acceptance.**
- `rg -n "First-run and backfill|Functions inside incremental bodies" docs/specs/incremental_models.md` returns two hits.
- `rg -n "incremental_models" docs/specs/functions.md docs/specs/planner_integration.md` shows the cross-references.

**Commit.** `spec: pin incremental first-run, partial-failure, and function-call semantics (review M9, M13)`

---

### Phase 16a: Pick canonical homes; promote scope-callout pattern

**Closes:** Mi6, Mi8, Mi17.

**Steps.**

1. **Mi17 — `columns:` canonical home in `models.md`.** Today the `columns:` frontmatter is described across `models.md`, `schema_evolution.md`, `data_catalog.md`, `testing.md`. Pick `models.md` as canonical. In `models.md` §"YAML frontmatter keys", add a §"`columns:` — column metadata" subsection that pins the full shape: per-column entries with `description`, `type` (link to `types.md`), `tests` (link to `testing.md`), `evolution` (link to `schema_evolution.md`), `tags`. In `schema_evolution.md`, `data_catalog.md`, `testing.md`: keep only the keys each spec normatively defines and open with "See `models.md` §`columns:` for the full shape."

2. **Mi6 — `data_catalog.md` "Tests" column normative definition.** Pin: the "Tests" column for a given model lists every test that targets it (test models with `test.model: <this model>` in their frontmatter), rendered as a bulleted list of test names linking to source. Add to `data_catalog.md` Surface §"Generated columns" (or wherever the column table lives); cross-link to `testing.md`.

3. **Mi8 — promote scope-callout pattern to `SPEC_TEMPLATE.md`.** Add a required header element (between frontmatter and "Surface"): a `> **What this is. ...**` blockquote stating the spec's scope and naming adjacent specs. Update `SPEC_TEMPLATE.md`'s example to match.

4. Bump `last_reviewed` on each touched spec.

**Acceptance.**
- `rg -n "^### .columns:." docs/specs/models.md` returns one hit.
- `rg -n "See .models\.md. § .columns:" docs/specs/` returns ≥3 hits.
- `SPEC_TEMPLATE.md` documents the scope-callout requirement.

**Commit.** `spec: pick canonical home for columns: and promote scope-callout pattern (review Mi6, Mi8, Mi17)`

---

### Phase 16b: Promote v1 sharp edges to Surface

**Closes:** Mi10, Mi11, Mi12, Mi13, Mi14, Mi15, Mi16, Mi18, Mi19.

**Source-of-truth call.** Each item is already documented somewhere; the fix is moving it from a buried location (Design / Known Divergences / docs-site only) to the Surface section of the spec a user reads first.

**Steps.**

1. **Mi10 — `unstable_schema:` discoverability.** In `smelt_yml.md` Surface §"`unstable_schema:`" (or add subsection), list every key currently gated by `unstable_schema: true` (today: `joins:`, `provenance:`). Note that an enumeration command (`smelt unstable list`) is open work; the static list is v1 source of truth.

2. **Mi11 — `smelt docs path` no-op.** In `cli.md` Surface §"Commands", flag `smelt docs path` as "stub: prints a message indicating docs are embedded; future feature." If it's not earning its slot, propose removing it; otherwise document the stubness inline.

3. **Mi12 — `smelt explain` test exclusion.** In `cli.md` Surface §"`smelt explain`" and `testing.md` Design where the claim originates, pin the mechanism. During execution, verify against `crates/smelt-cli` to determine actual flag (`--exclude-tests`, tag-based, or both). Document concretely; do not hand-wave.

4. **Mi13 — ephemeral seed size.** In `seeds.md` Surface §"Ephemeral seeds":
   > v1 has no row-count threshold for ephemeral seeds — declaring `materialization: ephemeral` on a 100k-row CSV will generate a `VALUES` literal of dangerous size. User judgement is the v1 control. A future warn-then-error threshold is open.

5. **Mi14 — strict CSV defaults.** In `seeds.md` Surface §"CSV format the loader accepts", add a callout that v1 has no per-seed override (no custom delimiter, no NULL marker, no quote char). Today this is in Design; promote.

6. **Mi15 — `--show-plan` positional required.** Already in `cli.md:69` Surface; verify the wording is prominent (paragraph, not parenthetical) and mention "no project-wide form" explicitly.

7. **Mi16 — `smelt build --dry-run` does not exist.** Already in `cli.md:74`; verify and promote to a top-level note in §"`smelt build` flags".

8. **Mi18 — `PASSING` context-sensitivity.** In `functions.md` Surface §"`PASSING`", add a worked example showing where `PASSING` is recognised (function-call argument list) vs where it's a regular identifier. Note that the parser distinguishes by position; this is intentional.

9. **Mi19 — compile/runtime CSV inference divergence.** In `seeds.md` Surface §"Type inference":
   > Compile-time inference samples the first 100 rows; runtime reads all rows. A type that's compatible with the sample but not the tail will be caught only at runtime.
   Cross-link to `lsp.md` for the LSP's compile-time view.

10. Bump `last_reviewed` on each touched spec.

**Acceptance.** Each item appears in the Surface section of its named spec, not buried.

**Commit.** `spec: promote v1 sharp edges to Surface (review Mi10–Mi16, Mi18, Mi19)`

---

### Phase 16c: Audit Design sections for rejected-alternatives paragraphs

**Closes:** Mi9.

**Source-of-truth call.** `feedback_specs_include_design.md` (user memory) requires Design sections to record rejected alternatives, not just restate Surface. Today only `architecture.md` and the typing quartet (`gradual_typing.md`, `types.md`, `functions.md`, `scoping.md`) clearly meet that bar.

**Steps.**

1. **Audit pass.** For each spec in `docs/specs/*.md`, read its `## Design` section. Mark it OK / NEEDS-WORK against the bar: every load-bearing design choice has a paragraph that names what was rejected and why. Append the audit list as a checklist to this plan (or a `docs/handoffs/` note) as the first commit on this phase.
2. **Easy fixes inline.** Where a Design paragraph already implies the rejected alternative, promote to explicit form: "We chose X. *Z was rejected* because…" Apply to NEEDS-WORK specs that need only one or two missing paragraphs.
3. **Defer hard cases.** Where a Design section needs substantial new content (rationale was never written), spawn a `/smelt:spec <feature>` follow-up rather than fabricating reasons. List spawned items in this plan's Spawned table.
4. Bump `last_reviewed` on every spec touched.

**Acceptance.** The audit list exists; every "easy fix" spec lands a real rejected-alternatives paragraph; hard cases are spawned, not faked.

**Commit.** `spec: Mi9 Design rejected-alternatives audit and inline fixes (review Mi9)`

---

### Phase 17: Anchor spawn list as in-spec Known Divergences

**Closes:** the spawn list — H6, H7, H8, M12, M14, plus 14c (journey integrity matrix). This phase does *not* substantively resolve those findings; it makes them visible inside the spec set so future plans citing those specs inherit the gaps explicitly.

**Source-of-truth call.** Self-documenting specs are higher-leverage than a sidecar backlog. Phase 13 already established the pattern (a Known Divergence in the relevant spec naming the future spec/plan). This phase generalizes it.

**Steps.**

1. **Phase 10 → `diagnostics.md` (new spec).** Add a Known Divergence in each of the seven specs that catalogue diagnostic codes:
   - `lsp.md`, `functions.md`, `gradual_typing.md`, `scoping.md`, `types.md`, `planner_integration.md`, `incremental_models.md`
   - Each entry, paraphrased per spec but sharing the template:
     > **Diagnostic codes pre-`diagnostics.md`.** Codes listed here are owned by this spec until a `diagnostics.md` spec lands. `diagnostics.md` will define ownership rules, severity tiers, stability tiers, and suppression. Code names may be renamed under that spec. (See `architecture.md` §"Specs not yet authored".)

2. **Phase 11 → `run_state.md` (new spec).** Add Known Divergences in:
   - `cli.md`:
     > **Manifest format and `.smelt/` layout pre-`run_state.md`.** Manifest format, `.smelt/` directory layout, run IDs, parallelism semantics, and failure recovery are not specified. `smelt status` and `smelt history` Surface descriptions in this spec name commands but defer their on-disk format to a future `run_state.md`. Behaviour is implementation-defined until then.
   - `incremental_models.md`: extend the existing "No interval / run-state tracking" entry to point at `run_state.md`.
   - `schema_evolution.md`: add a similar pointer for `.smelt/schemas/`.

3. **Phase 12 → multi-backend (new spec or architecture.md expansion).** Add a Known Divergence to `architecture.md` §"Backend trait surface":
   > **Multi-backend execution model not specified beyond trait surface.** Capability negotiation (incremental support, MERGE support, ALTER COLUMN support), cross-engine reference resolution rules (when does `read_parquet()` substitution apply?), and target precedence will land in `multi_backend.md` (or an expansion of this section). Today, capability claims are scattered across `incremental_models.md`, `schema_evolution.md`, `testing.md`, and `smelt_yml.md`.

4. **Phase 14c → journey integrity matrix.** Add a Known Divergence to `architecture.md` Constraints (or as its own §"Cross-cutting journey integrity"):
   > **User journey integrity matrix open.** The cross-product of testing × incremental × schema-evolution × multi-backend is not pinned end-to-end. Pinning depends on `run_state.md` and the multi-backend spec landing first.

5. **Phase 15 → dbt migration.** Add a Known Divergence to `architecture.md`:
   > **dbt comparison and migration story not specified.** Expected home: a `migration_from_dbt.md` spec or a dedicated docs-site/ guide. Until authored, the gap is a known limitation for adopters migrating from dbt.

6. **Phase 16-M14 → Decimal arithmetic.** Strengthen the existing `types.md` Known Divergence to pin the v1 fallback shape:
   > **Decimal arithmetic v1 fallback.** Decimal arithmetic in v1 produces `Decimal(38,10)` regardless of operand precision (e.g. `Decimal(19,2) + Decimal(19,2) → Decimal(38,10)`), where DuckDB native produces `Decimal(19,2)`. The fallback is conservative and avoids precision-loss; precision-aware inference is open.

7. **`architecture.md` anchor.** Add a new subsection at the end of `architecture.md` Known Divergences: §"Specs not yet authored":
   > **Specs not yet authored.** The spec set has explicit gaps that the following entries claim space for. Each names the in-scope future spec and which existing specs will pull content out of it:
   > - **`diagnostics.md`** — owns the diagnostic-code catalogue. Today scattered across `lsp.md`, `functions.md`, `gradual_typing.md`, `scoping.md`, `types.md`, `planner_integration.md`, `incremental_models.md`.
   > - **`run_state.md`** — owns manifest format, `.smelt/` layout, run IDs, parallelism, recovery. Today implicit in `cli.md` (`smelt status`, `smelt history`) and `incremental_models.md` (state ownership).
   > - **Multi-backend execution model** — likely an expansion of §"Backend trait surface" or a dedicated `multi_backend.md`. Today scattered across `incremental_models.md`, `schema_evolution.md`, `testing.md`, `smelt_yml.md`.
   > - **`planner_api.md`** — owns the user-authored planner-rule surface. Working design at `docs/planner_rule_api_design.md`; needs review against the 2026-05-01 universal-addressing rework before becoming normative.
   > - **`migration_from_dbt.md`** *(or docs-site guide)* — owns the dbt analogue mapping and migration story. No content today.
   > Each in-spec Known Divergence cross-references this anchor.

8. Bump `last_reviewed` on every touched spec.

**Acceptance.**
- `rg -n "diagnostics\.md|run_state\.md|multi_backend|planner_api\.md|migration_from_dbt\.md" docs/specs/` shows pointers in the relevant specs.
- `architecture.md` §"Specs not yet authored" lists all five planned specs.
- Every entry in this plan's "Spawned" table has a matching anchor.

**Commit.** `spec: anchor planned specs as in-spec Known Divergences (review H4 follow-on, H6, H7, H8, M9 follow-on, M12, M14)`

---

## Verification (after all phases land)

Before declaring this plan complete, run:

```sh
# 1. No legacy addressing in any spec
rg -n "smelt\.models\.|UndefinedModelRef|UndefinedSource" docs/specs/

# 2. No legacy paths config
rg -n "model_paths|seed_paths" docs/specs/

# 3. No stale future pointers
rg -n "when (written|authored)" docs/specs/

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

# 8. Spawned items anchored (Phase 17)
rg -n "diagnostics\.md|run_state\.md|multi_backend|planner_api\.md|migration_from_dbt\.md" docs/specs/

# 9. Architecture.md anchor exists
rg -n "Specs not yet authored" docs/specs/architecture.md
```

All nine should return the expected results.

Then:
- Re-read `docs/spec-review-2026-05-03.md` Headline-findings section. Every Critical and Major except those listed in the Spawned table should now be addressed (mostly resolved; H6/H7/H8/etc. anchored as Phase 17 work-to-be-done entries that future plans will pick up).
- Run `/smelt:validate` on the most-touched specs (`architecture.md`, `models.md`, `testing.md`, `lsp.md`, `incremental_models.md`) to surface implementation drift exposed by the spec changes.

## Notes for the agent executing this plan

- **Do not edit `docs/specs/` content beyond what each phase calls for.** The review and this plan's design calls are the oracle; if neither names something, defer it.
- **Bump `last_reviewed: 2026-05-04` on every touched spec.** This is the audit signal future reviewers use.
- **Anchor cross-references where possible** (`architecture.md#resolution`) instead of bare filename. Phase 8 step 7 codifies this for new cross-references.
- **Commit-and-push at each phase boundary** so the user sees progress on the tracking PR.
- **No code changes in this plan.** Verification commands are read-only `rg` / `grep`. If a spec change implies a code change, note it in the phase commit message and let `/smelt:validate` surface the drift.
- **Phase 9 step 4 and Phase 14b step 5 require reading code** (`crates/smelt-core/src/metadata.rs` for within-file duplicate handling; `crates/smelt-planner/src/rules/incremental.rs` for opaque-call classification). Read-only — pin spec wording to actual behaviour.
- **Phase 17 is purely additive** — new Known Divergence entries, no edits to existing surface. Safest phase to land last.
