# Spec review — 2026-05-03

**Date:** 2026-05-03
**Scope:** all 22 normative specs in `docs/specs/` (plus `SPEC_TEMPLATE.md`), ~4,300 lines authored or last-reviewed in the past ~2 weeks.
**Method:** three independent reviewer subagents with distinct lenses, each writing structured findings; this document is the synthesis.

> **Why this review exists.** `docs/specs/` is now the canonical answer to "how does feature X work?" — drift, contradiction, or under-specification compounds via the leverage hierarchy in `CLAUDE.md` (1× into code, 100× into plans, 1000× into the next round of specs). This review's purpose is to catch that drift before the next implementation plan cites these specs.
>
> The deliverable is a review, not a re-spec. Findings are actionable (file, section, suggested fix), prioritised by severity, and traceable so each can become a follow-up plan. **No specs are modified by this PR.**

---

## Executive summary

The spec set is **mid-migration and not yet self-consistent**. `architecture.md` (the only `status: stable` spec) was substantially reworked on 2026-05-01 to introduce three load-bearing inventions:

1. **Universal `smelt.<path>` addressing** for every project-defined entity.
2. **Models-as-functions** (transparency vs materialization as orthogonal axes).
3. **Unified `paths:` scan list** in `smelt.yml`.

The thesis is strong — the typing quartet (`types.md` / `gradual_typing.md` / `functions.md` / `scoping.md`) and `architecture.md` show what "good" looks like. But six feature specs still describe the *previous* world (kind-prefixed `smelt.models.<name>`, separate `model_paths`/`seed_paths`), and a duplicate spec (`project_config.md`) coexists alongside its replacement (`smelt_yml.md`) with mutually contradictory schemas — both dated 2026-05-03, both claiming normative authority.

Three reviewers — vision, consistency, and practitioner — independently converged on the same top issues. That convergence is signal: these are not stylistic quibbles, they are the spec set's load-bearing failures right now.

**Top 5 actions, in order:**

1. **Pick one config spec and delete the other.** `smelt_yml.md` matches the implementation, examples, and docs-site. Delete `project_config.md` (or shrink it to a redirect note salvaging only the cross-engine Parquet-exchange paragraph).
2. **Run the addressing-scheme migration to completion.** Sweep `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md` to use `smelt.<path>`. Rename the `UndefinedModelRef` family of diagnostic codes accordingly.
3. **Resolve the test-declaration split.** `architecture.md` introduces `smelt.test <name>` as a top-level declaration; `testing.md` only specifies `materialization: test`. Pick one and delete (or label) the other across `architecture.md`, `functions.md`, `models.md`, `testing.md`.
4. **Add the missing system-level specs the existing set implies but doesn't cover:** `diagnostics.md` (the error-code catalogue), `run_state.md` (manifest / `IntervalStore` / `.smelt/` lifecycle), and either `multi_backend.md` or an expansion of `architecture.md` §"Backend trait surface" into a full multi-backend execution model.
5. **Lift the planner-extensibility differentiator into a public surface.** Either pull `docs/planner_rule_api_design.md` into `docs/specs/planner_api.md` as a stub, or add an explicit Known Divergence to `planner_integration.md` naming this as scoped future work. Today it reads as if smelt is a closed pipeline of four hardcoded rules — undermining the headline differentiator from `README.md` and `CLAUDE.md`.

After those five, the secondary cleanup (~12 Major findings) becomes mechanical: consistent unknown-key handling, dropping stale "(when written)" markers for `expansion.md` / `tests.md`, fixing the `smelt-optimizer` → `smelt-planner` references in `incremental_models.md`, etc.

**Verdict by reviewer:**

| Lens | Verdict |
|---|---|
| Vision & coherence | Strong thesis; rest of spec set hasn't caught up to the rework. Two thirds excellent, one third lagging. |
| Consistency & structural | Template conformance excellent; cross-spec terminology drift severe. Migration incomplete. |
| Practitioner adoption | **Wait.** Cannot recommend to a team until `smelt.yml` and addressing are pinned. |

---

## Scope and method

- **Inputs:** all 22 specs in `docs/specs/` plus `SPEC_TEMPLATE.md`.
- **Reviewers:** three parallel subagents, each given a tight remit:
  1. **Vision & coherence** — does the spec set articulate the smelt thesis, anchor on `architecture.md`, cover the differentiators, compose end-to-end?
  2. **Cross-spec consistency & structural** — terminology drift, conflicting rules, dangling refs, SPEC_TEMPLATE conformance, frontmatter hygiene.
  3. **Practitioner / user-surface** — could a senior dbt user build with these specs alone? Surface specificity, edge-case gaps, ergonomics, migration story.
- **Output:** each lens wrote a structured findings file (severity-tagged, actionable). This synthesis dedupes and ranks. Full per-lens reports are in the **Appendices** — synthesis points cite them as `[V]`, `[C]`, `[P]`.
- **Severity buckets:**
  - **Critical** — blocks correct implementation or undermines a thesis-level claim.
  - **Major** — real gap or inconsistency users / implementers will hit.
  - **Minor** — polish, clarity, missing rationale.
  - **Nit** — typos, formatting.
- **Verification (per the plan):** every finding below traces back to an explicit per-lens finding (no synthesised invention). Two convergent findings are marked `[V][C][P]`; single-lens findings carry one tag. Severity is reconciled when lenses disagree (noted inline). Spot-checked three findings against the cited spec sections.

---

## Headline findings

These are the issues all three lenses surfaced or that block the most downstream work. Each is a Critical from at least one lens.

### H1. `smelt.yml` is specified twice with contradictory schemas `[V][C][P]`

**Files:** `docs/specs/smelt_yml.md`, `docs/specs/project_config.md` — both `last_reviewed: 2026-05-03`, both claim normative authority, neither cites the other.

| Field | `smelt_yml.md` | `project_config.md` |
|---|---|---|
| Path config | unified `paths: ["models"]` | separate `model_paths` + `seed_paths` |
| Unknown keys | warning | silently ignored |
| `default_materialization` values | includes `test` | omits `test` |
| Targets fallback | (no entry) | "Unknown target types fall back to DuckDB" |
| `version` field | required, integer | optional, "decorative" |

The implementation, every example workspace, the docs-site reference page, and the `smelt_yml.md` Design rationale all use the unified `paths:` shape. `project_config.md` reads like an older code-anchored description. Yet six other specs (`models.md`, `python_models.md`, `testing.md`, `cli.md`, plus the dependent specs) reference `model_paths` / `seed_paths` — i.e., the side that does not match the code.

**Fix:** Delete `project_config.md` (or shrink to a redirect). Salvage only its cross-engine Parquet-exchange paragraph into `architecture.md` §"Backend trait surface" or a future `multi_backend.md`. Then sweep the four downstream specs to use `paths:`.

### H2. Universal `smelt.<path>` addressing migration is incomplete `[V][C][P]`

**Anchor (correct):** `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme" + Design §"Single addressing scheme `smelt.<path>` for all project-defined entities". This is the load-bearing claim.

**Specs still using legacy `smelt.models.<name>` / `smelt.sources.<schema>.<table>`:**
- `models.md` Surface §"Reference syntax" — only the legacy form documented.
- `lsp.md` — every diagnostic name (`UndefinedModelRef`, `UndefinedSource`), every goto-definition row, every completion trigger.
- `python_models.md` — examples and §"Model name derivation".
- `testing.md` — `inputs:` keys, mock substitution rules.
- `model_selection.md` — graph traversal description.
- `data_catalog.md` — diagnostic surface (inherits from `lsp.md`).

The Critical here is not "this is wrong" — it is "`architecture.md` is right and the rest of the spec set hasn't caught up." A reader cannot tell which form is current. An LSP implementer following `lsp.md` will produce different completions than an architecture-conformant implementation. Tutorial authors following `models.md` will teach the wrong syntax.

**Fix:** Single spec-only PR replacing legacy addresses. Rename `UndefinedModelRef` to `UnknownSmeltPath` (matching `functions.md`'s `UnknownSmeltFn` family). Add a one-line migration-date note to `architecture.md` §"Resolution" so future reviewers can see the cutover landed. Update `examples/timeseries/` to demonstrate model→model references.

### H3. Test declaration is described two ways across three specs `[V][C]`

- `architecture.md` Known Divergences (line 324): `smelt.test <name>` is a top-level declaration kind alongside `smelt.define` / `smelt.extern` — assertion semantics deferred to "a future `tests.md`".
- `functions.md` §"File structure": `smelt.test` listed as a top-level item.
- `models.md` §"Materialization modes": `materialization: test` on a regular model, with a `test:` frontmatter object.
- `testing.md` Surface: documents `materialization: test` form **only**, no mention of `smelt.test` declarations.

These are two distinct designs. `materialization: test` is what the code implements (per `testing.md` References pointing at `crates/smelt-core/src/metadata.rs::TestConfig`); `smelt.test <name>` appears aspirational. Plus: `architecture.md` defers to a "future `tests.md`" file that is actually `testing.md` and already exists.

**Fix:** Pick one. Recommended — keep `materialization: test` (matches code), drop `smelt.test` from `architecture.md` and `functions.md`, rename `architecture.md`'s "future `tests.md`" pointer to "`testing.md`". If `smelt.test` is also intended to land, document both forms in `testing.md` with a clear "current vs future" framing.

### H4. The "engineer controls the planner" differentiator has no public surface `[V]`

`README.md` and `CLAUDE.md` frame planner extensibility as a top-five differentiator from dbt — "the API will allow data engineers to refactor specific logical plans." The spec set says nothing about how:

- No public API spec for `PlannerRule`. The trait is mentioned exactly once in `planner_integration.md` Constraints ("Every `impl PlannerRule` must be `Send + Sync`").
- No loading mechanism (Rust crate? Python rule? config-time enable?).
- No discovery / registration / lifecycle / stability story.
- No future-work entry naming this as a planned spec.

A reader of only the specs would conclude the planner is a closed pipeline of four hardcoded L1 rules. That's the dbt-clone framing, not the smelt thesis.

**Fix:** Either (a) add a Known Divergences entry to `planner_integration.md` saying "user-authored planner-rule API is in scope but pre-spec," or (b) pull `docs/planner_rule_api_design.md` into `docs/specs/planner_api.md` as a stub. Option (b) preferred — it puts the differentiator on the spec map even if everything in it is marked unstable.

### H5. End-to-end user journey breaks at testing ↔ incremental ↔ schema-evolution ↔ multi-backend joins `[V][P]`

Walking the canonical journey ("define typed model, run incrementally on Spark, evolve schema, test it"), the spec set has four known-but-unflagged gaps:

1. **Incremental on Spark is pre-spec.** `incremental_models.md` Known Divergences flags the Spark MERGE pathway as unbuilt — but `models.md` lists `incremental:` as a frontmatter key without target restrictions.
2. **Tests always run on DuckDB.** `testing.md` Constraints §1 — Spark-only projects cannot test their own models. Buried in Known Divergences, not framed as a Design-section trade-off.
3. **Schema evolution + incremental interaction is unspecified.** `incremental_models.md` Known Divergences: "Schema evolution is unspecified. A `partition_column` rename or output schema change has no defined handling today." `schema_evolution.md` does not mention incremental models or partition columns at all.
4. **Multi-target precedence is fragile.** `smelt_yml.md` Known Divergences: whether a model can pin a target not declared in `targets:` is open.

**Fix:** Add a "User journey integrity matrix" to `architecture.md` Constraints (or a small `cross_cutting.md`) naming the journeys the set claims to support and the gaps each currently has. Specifically: in `models.md`, mark `incremental:` as DuckDB-only today. In `testing.md` Design, elevate "DuckDB-only test runtime" from divergence to deliberate trade-off. Open a section in `schema_evolution.md` covering the partition-column case (or stub `incremental_schema_evolution.md`).

### H6. No spec covers the diagnostic-code catalogue `[V][C][P]`

Diagnostic codes are listed in **six** specs (`functions.md`, `gradual_typing.md`, `scoping.md`, `lsp.md`, `types.md`, `planner_integration.md`) with overlapping but non-identical descriptions and no canonical home. `lsp.md` calls one code `UndefinedModelRef`; `functions.md` doesn't catalogue any model-ref code; `lsp.md` doesn't mention `MissingProvenancePushdownAdvisory` from `planner_integration.md`. Severity (Error / Warning / Hint) is fragmented. There is no stability story.

**Fix:** Add `docs/specs/diagnostics.md` listing every code, severity, anchor rule, stability tier, and the spec that owns it. Other specs link to it and add only the single-line "triggered by …" description for codes they own. Rule: each code is *owned* by exactly one spec; others may reference but must not redefine the trigger.

### H7. No spec covers run-state / build orchestration / observability `[V][P]`

`cli.md` lists `smelt status [model]` and `smelt history [model]`. `architecture.md` Crate table mentions `RunManifest`, `IntervalStore`, `FileStore`. `schema_evolution.md` mentions `.smelt/schemas/`. **No spec covers** the data model behind these — what's on disk, when written, format compatibility, run IDs, parallelism, failure recovery, log format, log shape on compiled-SQL emission, etc.

**Fix:** Add `docs/specs/run_state.md` (or `build_lifecycle.md`) covering the manifest format, `.smelt/` layout, log/output format, and what each subcommand reads/writes. This is dbt's `manifest.json` story; the gap is glaring once you look. Also: `CLAUDE.md` says "we want to push towards production ready" — this is the area least ready to back the claim.

### H8. No spec covers the multi-backend execution story end-to-end `[V][P]`

Backend selection lives in `models.md` (`target:` frontmatter) and `smelt_yml.md` (precedence table). The cross-engine Parquet-exchange story is two paragraphs in `project_config.md` (the spec we want to delete, per H1). Capability matrices are scattered. No spec answers: when the planner sees a model on backend A reading from a model on backend B, what are the rules? Where does `read_parquet` substitution happen? When does it not work (upstream is a view)? What about Databricks-specific features?

**Fix:** Either expand `architecture.md` §"Backend trait surface" into a full §"Multi-backend execution model" (preferred), or create `multi_backend.md`. Salvage the Parquet-exchange paragraph from `project_config.md` here.

---

## Findings by severity

A finding tagged `[V][C][P]` was raised by all three reviewers; `[V][C]` by vision and consistency; etc. Severity is the highest assigned by any lens, with reconciliation noted inline.

### Critical

| ID | Title | Lens | Affected specs |
|---|---|---|---|
| H1 | Two contradictory `smelt.yml` specs | `[V][C][P]` | `smelt_yml.md`, `project_config.md`, all that reference either |
| H2 | `smelt.<path>` addressing migration incomplete | `[V][C][P]` | `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md` |
| H3 | Test declaration described two ways | `[V][C]` | `architecture.md`, `functions.md`, `models.md`, `testing.md` |
| H4 | Planner-extensibility differentiator has no public surface | `[V]` | `planner_integration.md` |
| H5 | Cross-cutting user journey broken | `[V][P]` | `incremental_models.md`, `testing.md`, `schema_evolution.md`, `models.md`, `smelt_yml.md` |
| C1 | Multi-model file format described two ways | `[C]` | `architecture.md` (per-decl `name:` frontmatter) vs `models.md` / `python_models.md` / `testing.md` (`--- name: X ---` section delimiter) |

C1 detail: `architecture.md` §"Bare-model naming" says multi-model files use per-declaration YAML frontmatter with a `name:` key. `models.md` §"File format" says they use `--- name: <model_name> ---` section delimiters and "any other `--- X ---` form is a hard parse error." A parser cannot satisfy both rules. Plans for multi-model files will diverge depending on which spec they cite. Decide which is normative — and if section-delimiter is the rule, `architecture.md` §"Bare-model naming" needs replacing.

### Major

| ID | Title | Lens | Affected |
|---|---|---|---|
| H6 | No diagnostic-code catalogue / stability surface | `[V][C][P]` | system-level gap; touches 6 specs |
| H7 | No run-state / build-lifecycle / observability spec | `[V][P]` | system-level gap |
| H8 | No multi-backend execution model spec | `[V][P]` | system-level gap |
| M1 | `expansion.md` exists but referenced as "(when written)" 5×  | `[C]` | `gradual_typing.md`, `planner_integration.md`, `scoping.md` |
| M2 | `tests.md` referenced as future, but `testing.md` exists | `[C]` | `architecture.md`, `functions.md`, `seeds.md`, `sources.md` |
| M3 | `smelt-optimizer` crate cited in `incremental_models.md` does not exist (it's `smelt-planner`) | `[C]` | `incremental_models.md` |
| M4 | `incremental_models.md` and `types.md` lack a `## Design` section | `[C]` | per `SPEC_TEMPLATE.md` and user's recorded preference, this is required |
| M5 | Unknown-key handling differs across specs (hard error / warning / silent / mixed) | `[V][C]` | `models.md` (error), `smelt_yml.md` (warning), `project_config.md` (silent), `functions.md` (warning) |
| M6 | `cli.md` lifecycle references `seed_paths` and aggregate `sources.yml` (both retired) | `[C]` | `cli.md` |
| M7 | `lsp.md` cites `sources.yml` (single file, retired) instead of per-entity source `.yml`s | `[C]` | `lsp.md` |
| M8 | `models.md` describes `model_paths`; reality is unified `paths:` | `[C]` | `models.md`, `python_models.md`, `testing.md` |
| M9 | First-run, partial-failure, and out-of-order semantics for incremental models are unspecified | `[P]` | `incremental_models.md` |
| M10 | `smelt build` flag table omits schema-evolution flags (`--allow-column-removal`, `--allow-full-refresh`) | `[P]` | `cli.md` vs `schema_evolution.md` |
| M11 | `smelt test --select` is substring-match, not selector grammar — asymmetric | `[P]` | `model_selection.md`, `cli.md` Known Divergences; not flagged in `testing.md` |
| M12 | dbt migration / analogue mapping entirely missing | `[P]` | none of the 22 specs mentions dbt anywhere |
| M13 | Function calls inside incremental bodies — pushdown safety vs optimisation distinction unclear | `[P]` | `functions.md`, `incremental_models.md`, `planner_integration.md` |
| M14 | Type system × backend interaction hand-wavy on Decimal precision arithmetic | `[P]` | `types.md` Known Divergences |
| M15 | Selector edge cases unresolved (empty selection, `--exclude` orphaning, cross-`paths:` conflict) | `[P]` | `model_selection.md` |
| M16 | README differentiator list (5 rows) vs CLAUDE.md differentiator list (6) don't match | `[V]` | `README.md`, `CLAUDE.md` |

Severity reconciliation: M9 was Major in `[P]` — agreed. H6 was Major in `[V][P]` and Major in `[C]` (with overlap with M5), promoted to headline because three lenses surfaced it. The `[V]` Major "no run-state spec" and the `[V]` Major "no multi-backend spec" were both promoted to headlines (H7, H8) because they each name a missing system-level spec, not just a fix to existing ones.

### Minor

| ID | Title | Lens |
|---|---|---|
| Mi1 | `architecture.md` is `status: stable`; everything else `experimental` — `stable` is undefined in `SPEC_TEMPLATE.md` | `[C]` |
| Mi2 | `last_reviewed` dates lag — 7 specs predate the 2026-05-01 addressing rework and may not have been audited under the new rules | `[C]` |
| Mi3 | `sources.md` ↔ `seeds.md` mutual cross-reference points at non-existent headings | `[C]` |
| Mi4 | `models.md` §"Reference syntax" omits `smelt.define`-as-callable / parameterised-model surface | `[C]` |
| Mi5 | `lsp.md`'s `MalformedSource` / `SourceTypeError` codes are not reflected in `sources.md` | `[C]` |
| Mi6 | `data_catalog.md` markdown column-table promises a "Tests" column whose contents are not normatively defined | `[C]` |
| Mi7 | Tag case-sensitivity rule lives in `model_selection.md`; tag merging lives in `models.md` — they should colocate | `[C]` |
| Mi8 | `gradual_typing.md` / `types.md` / `functions.md` / `scoping.md` are an exemplar of "good" spec relationships — call this out as the bar | `[V]` |
| Mi9 | Most Design sections are restatement; only `architecture.md` and the typing trio capture rejected alternatives. User memory `feedback_specs_include_design.md` requires this. | `[V]` |
| Mi10 | `unstable_schema:` opt-ins are not enumerable from one place — no `smelt unstable list` or central catalogue | `[P]` |
| Mi11 | `smelt docs path` is a no-op stub | `[P]` |
| Mi12 | `smelt explain`'s test-exclusion mechanism is hand-waved | `[P]` |
| Mi13 | Ephemeral seed size limit is an open question — no warn-then-error today | `[P]` |
| Mi14 | Strict-CSV defaults (no per-seed delimiter / NULL marker / quote char overrides) — flagged in Design but should be visible Quickstart-level | `[P]` |
| Mi15 | `--show-plan` requires a positional model file (no whole-graph form) | `[P]` |
| Mi16 | `smelt build --dry-run` doesn't exist; only `smelt run --dry-run` does | `[P]` |
| Mi17 | `columns:` frontmatter is split across `models.md`, `schema_evolution.md`, `data_catalog.md`, `testing.md` — no canonical home | `[P]` |
| Mi18 | `PASSING` is a context-sensitive keyword; user docs need a worked example | `[P]` |
| Mi19 | Compile-time vs runtime CSV inference can diverge — sharp edge, deferred to LSP plan | `[P]` |

### Nit

| ID | Title | Lens |
|---|---|---|
| N1 | `incremental_models.md` Semantics mixes `smelt.models.<name>` and `smelt.<path>` within the same spec | `[C]` |
| N2 | `incremental_models.md` references `smelt-yml.md` (hyphen) — file is `smelt_yml.md` (underscore). Broken link. | `[C]` |
| N3 | References blocks shape varies — some specs use sub-headings, others flat bullets. Pick one in `SPEC_TEMPLATE.md`. | `[C]` |

---

## Findings by spec (cross-reference)

For each of the 22 specs: severity counts and a one-line summary. Specs with **zero findings** are listed at the bottom (proof of audit coverage).

| Spec | Crit | Maj | Min | Nit | One-line summary |
|---|---|---|---|---|---|
| `architecture.md` | 2 | — | 1 | — | Anchor spec, in good shape; carries H3 (`smelt.test`) and C1 (multi-model) and Mi1 (`stable` undefined). |
| `cli.md` | 1 | 2 | — | — | H5 (journey gaps surface here as flag-table omissions), M6 (legacy refs), M10 (missing schema-evo flags). |
| `data_catalog.md` | 1 | — | 1 | — | H2 (legacy addressing), Mi6 (`Tests` column undefined). |
| `datagen.md` | — | — | — | — | Clean. |
| `expansion.md` | — | — | — | — | Substantive and consistent; ironic given M1 is "other specs flag this as future." |
| `functions.md` | 2 | 1 | — | — | H3, H6 (diagnostic-code home), M2 (cites future `tests.md`). Otherwise excellent. |
| `gradual_typing.md` | — | 1 | 1 | — | M1 (cites future `expansion.md`), Mi8 (positive — exemplar). Strong spec. |
| `incremental_models.md` | 1 | 4 | — | 2 | H5, M3 (`smelt-optimizer`), M4 (no Design), M9 (first-run/partial-failure), M13 (functions interaction), N1 (mixed addressing), N2 (hyphen typo). Most-cited spec in findings. |
| `lsp.md` | 1 | 2 | 1 | — | H2, M7 (`sources.yml` legacy), Mi5 (codes not in `sources.md`). |
| `model_selection.md` | 1 | 2 | 1 | — | H2, M11 (`smelt test` selector asymmetry), M15 (edge cases), Mi7 (tag rules colocation). |
| `models.md` | 2 | 2 | 1 | — | H2, C1, M5 (unknown keys), M8 (`model_paths`), Mi4 (reference-syntax surface). High-impact spec; most user-facing. |
| `planner_integration.md` | 1 | 2 | — | — | H4, M1, H6. Otherwise excellent. |
| `project_config.md` | 1 | 1 | — | — | H1. **Recommended action: delete.** |
| `python_models.md` | 1 | 2 | — | — | H2, M8, Mi19 (compile/runtime CSV inference, indirectly via seeds). |
| `schema_evolution.md` | 1 | — | — | — | H5 (interaction with incremental). Otherwise solid. |
| `scoping.md` | — | 1 | 1 | — | M1, Mi8 (positive). |
| `seeds.md` | — | 1 | 2 | — | M2, Mi3 (mutual ref), Mi13 (ephemeral size), Mi14 (CSV defaults), Mi19 (inference divergence). One of the strongest specs overall — practitioner called it "the best-written spec in the set." |
| `smelt_yml.md` | 1 | 1 | — | — | H1, M5. **Recommended action: keep — declared canonical.** |
| `sources.md` | — | 1 | 1 | — | M2, Mi3. Clean otherwise. |
| `testing.md` | 2 | 2 | 1 | — | H2, H3, M11, Mi12 (`smelt explain` exclusion). |
| `types.md` | — | 2 | — | — | M4 (no Design), M14 (Decimal arithmetic). Despite being a strong spec, missing Design section is a structural gap. |
| `SPEC_TEMPLATE.md` | — | — | 1 | 1 | Mi1 (define `stable`), N3 (References shape). Should grow as the spec set's needs become clearer. |

**Specs with zero findings:** `datagen.md`, `expansion.md`. Verified by reading both end-to-end during synthesis — these are clean.

---

## Recommendations

Sequenced cleanup. Each PR is independent of later PRs, allowing them to land in any order — but the order below maximises early payoff.

### PR-1: Delete `project_config.md` and complete the addressing migration *(closes H1, H2, M5, M6, M7, M8, N1, N2, parts of Mi3)*

Single spec-only PR. Mechanical sweep:
1. Delete `project_config.md`. Salvage cross-engine Parquet-exchange paragraph into `architecture.md` §"Backend trait surface" (will become H8's seed).
2. Replace every `model_paths` / `seed_paths` reference with `paths:` across `models.md`, `python_models.md`, `testing.md`, `cli.md`.
3. Replace every `smelt.models.<name>` / `smelt.sources.<...>` legacy address with `smelt.<path>` form across `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md`.
4. Rename `UndefinedModelRef` family in `lsp.md` to `UnknownSmeltPath` (matching `functions.md`'s `UnknownSmeltFn`).
5. Add a one-line note to `architecture.md` §"Resolution" pinning the migration date.
6. Bump `last_reviewed` on every touched spec.
7. Update `examples/timeseries/` to demonstrate model→model references using the new addressing.

This is mechanical work but unblocks every other downstream cleanup.

### PR-2: Resolve test-declaration split and stale "future spec" pointers *(closes H3, M1, M2)*

Decide test-declaration shape (recommended: `materialization: test` only — matches code) and update:
- `architecture.md` Known Divergences — drop `smelt.test` entry.
- `functions.md` §"File structure" — remove `smelt.test` line.
- All five `(when written)` / `(planned)` markers for `expansion.md` — drop them.
- All four "future `tests.md`" pointers — rewrite as `testing.md`.
- Add explicit acknowledgement to `testing.md` Surface that the file is what was previously called `tests.md` in cross-references.

### PR-3: Add the three system-level missing specs *(closes H6, H7, H8, partial M5, M14)*

Three new specs:
- `diagnostics.md` — single canonical catalogue of every diagnostic code, severity, anchor, ownership, stability tier. Other specs link to this and only describe trigger conditions for codes they own. Resolves the `Mi5` / overlap issues automatically as a side effect.
- `run_state.md` (or `build_lifecycle.md`) — `RunManifest`, `IntervalStore`, `FileStore`, `.smelt/` layout, log/output format. Needed for production-readiness.
- `multi_backend.md` (or expand `architecture.md` §"Backend trait surface") — backend selection, cross-engine reference resolution, Parquet handoff, capability negotiation. Salvages content from PR-1's `project_config.md` deletion.

These can be stubs initially — the value is putting the differentiators on the spec map.

### PR-4: Add planner extensibility surface *(closes H4)*

Either pull `docs/planner_rule_api_design.md` into `docs/specs/planner_api.md` as a stub, or add a Known Divergences entry to `planner_integration.md`. Recommended: stub. Mark every section unstable. The thesis-level claim deserves a spec-map entry even if the API is still in flux.

### PR-5: Pin journey integrity and edge cases *(closes H5, M9, M10, M11, M13, M15, several Minor)*

Per-spec edits:
- `models.md`: mark `incremental:` as DuckDB-only today; cross-link to `incremental_models.md` Known Divergences.
- `incremental_models.md`: add Semantics §"First-run and backfill" covering chunking, transaction boundaries, partial-failure behaviour, late-arrival handling. Add §"Functions inside incremental bodies" covering pushdown safety vs optimisation.
- `testing.md` Design: elevate "DuckDB-only test runtime" from divergence to deliberate trade-off. Surface the `--select` substring-match deviation prominently in Surface, not buried elsewhere.
- `cli.md`: Add `--allow-column-removal`, `--allow-full-refresh` to the `smelt build` flag table (or split a "common flags" subsection).
- `model_selection.md`: pin empty-working-set behaviour (recommended: warning + exit 0), pin `--exclude` orphaning behaviour.
- `schema_evolution.md`: add a section on partition-column rename / type-change in incremental models, or stub `incremental_schema_evolution.md`.

### PR-6: Add Design sections, polish, dbt migration *(closes M4, M12, M16, most Minor)*

- Add `## Design` to `incremental_models.md` and `types.md`. Extract rationale from existing Constraints / scattered notes.
- Add a "dbt comparison" section to `architecture.md` (or stub `migration_from_dbt.md`) — one-line analogue mapping for `ref()`, `source()`, `is_incremental()`, `--full-refresh`, `dbt seed`, `dbt test`, exposures, freshness.
- Reconcile `README.md` and `CLAUDE.md` differentiator lists.
- Pass through Design sections to add at least one rejected-alternative paragraph each (use `architecture.md` and `gradual_typing.md` Design as the bar).
- Define `status:` allowed values in `SPEC_TEMPLATE.md` (esp. `stable`).

### Meta: SPEC_TEMPLATE evolution

Two structural changes worth considering:

1. **Open every spec with a "Scope" callout** naming the adjacent specs that own related things (positive pattern from the typing quartet — `gradual_typing.md` already does this and it works well).
2. **Cross-spec link discipline** — encourage anchored references (`architecture.md#resolution`) over file-only references, so heading renames don't silently break cross-spec navigation. Could be enforced by `/smelt:validate`.

---

## Risks and limitations of this review

- **No code consulted beyond crate-name verification.** This is a review *of the specs*, not of implementation drift against them — that is `/smelt:validate`'s job. Several findings imply the implementation may already be ahead of (or behind) the spec; that's not assessed here.
- **No user-doc audit.** `docs-site/` was sampled lightly. Several findings refer to user-doc/spec drift but don't enumerate it. A separate user-doc audit is warranted after PR-1 lands.
- **Vision lens sampled three specs (`functions.md`, `scoping.md`, `seeds.md`)** rather than reading them in full, on the basis their thesis-relevant content overlaps with adjacent specs. The consistency lens read all 22 in full. The practitioner lens read all 22 in full. Coverage of those three should be considered "verified by two of three lenses" rather than three.
- **Findings are based on the specs as of 2026-05-03.** The spec set is moving fast; some findings may already be in flight in branches not visible at review time.

---

## Appendix A: Vision & coherence raw report

> Reviewer: vision-lens subagent. Findings: 4 Critical, 4 Major, 3 Minor. Synthesis findings cite back to it via `[V]` tags.

**Specs reviewed:** all 22 in `docs/specs/`. Read in full: `architecture.md`, `planner_integration.md`, `models.md`, `types.md`, `gradual_typing.md`, `incremental_models.md`, `schema_evolution.md`, `lsp.md`, `cli.md`, `smelt_yml.md`, `project_config.md`, `python_models.md`, `testing.md`, `expansion.md`, `data_catalog.md`, `model_selection.md`, `datagen.md`, `sources.md`. Sampled (header + key sections): `functions.md`, `scoping.md`, `seeds.md`. The "sampled" specs were skimmed because their thesis-relevant content overlaps heavily with what the architecture / planner / models specs already establish; nothing in their sampled portions contradicts the findings below.

### A.Summary

- **Two parallel addressing schemes coexist in the spec set, and neither is marked as "the new one wins".** `architecture.md`, `functions.md`, `sources.md`, `seeds.md`, `scoping.md` all say `smelt.<path>` is universal. `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md` still use `smelt.models.<name>` and `smelt.sources.<schema>.<table>`. Pick one — anything else undermines the load-bearing claim that addressing falls out of structure.
- **Two parallel project-config specs exist (`smelt_yml.md` and `project_config.md`) and they disagree on basic facts.** `paths` vs `model_paths`+`seed_paths`; `name` required vs decorative; `version` required vs optional; unknown keys = warning vs silently ignored. One must be deleted (or recast as user-doc reference) — having two normative specs for the same surface is worse than having none.
- **The "engineer controls the planner" differentiator is the project's most distinctive claim, and it is the most under-claimed thing in the spec set.** `planner_integration.md` is excellent on what the four shipped L1 rules do today, but contains no public planner API, no rule registration story, no extension point. The `RuleContext` / `PlannerRule` trait is mentioned exactly once and treated as internal. If "data engineers refactor specific logical plans" (per `CLAUDE.md` and `README`) is real, it deserves either a spec section, a future-work entry with shape, or a separate `planner_api.md` stub. Right now it reads as if it isn't part of the project.
- **Architecture.md is genuinely doing the anchor job** — it's referenced from at least 12 other specs and pins the load-bearing invariants (Rowan-as-only-IR, transparency vs materialization orthogonality, models-as-functions, universal addressing, value-producing planner, sync core / async edges). That's the foundation for everything else; the issue is that several feature specs ignore it.
- **A coherent end-to-end user journey** ("define typed model, run incrementally on Spark, evolve schema, test it") **does NOT compose cleanly from the specs as written** — see Critical 4 below.

### A.Findings

#### A.Critical 1: Two contradictory addressing schemes claimed simultaneously

- **Spec(s):**
  - `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme" — declares `smelt.<path>` universal; claims model addresses follow the path-with-scan-root-stripped rule (e.g., `smelt.marts.customers`, `smelt.raw.events`, `smelt.tests.marts.customers_no_nulls`).
  - `models.md` §"Reference syntax" — `smelt.models.<name>` only. `smelt.models.<name>(filter => …)` shown as the parameterised form.
  - `lsp.md` §"Diagnostic categories", §"Go-to-Definition", §"Find References", §"Hover", §"Completions" — every reference site uses `smelt.models.<name>` and `smelt.sources.<schema>.<table>`. `UndefinedModelRef` and `UndefinedSource` are the diagnostic codes.
  - `python_models.md` examples and §"Model name derivation" — `smelt.models.combined_events`, `smelt.models.daily_revenue`.
  - `testing.md` §"Whole-model tests" — "all `smelt.models.<name>` and `smelt.sources.<name>` references".
  - `model_selection.md` §"Graph traversal" — `smelt.models.<name>` and `smelt.sources.<name>`.
  - `sources.md` §"Filesystem layout" example — `smelt.sources.raw.users` (the `sources` segment here is the directory name, not a kind prefix; this is consistent with `architecture.md` but reads identically to the `models.md`-style "kind prefix" addressing, which is exactly the conflation the universal scheme was meant to remove).
- **Observation:** This is not a small surface-level inconsistency — it is the load-bearing claim of `architecture.md` §"Resolution" being silently revoked by half the feature specs. The Design section of `architecture.md` ("Single addressing scheme `smelt.<path>` for all project-defined entities") explicitly rejects kind-prefixed addressing as a *previous* shape that was wrong. Yet five other specs still use that previous shape. A reader cannot tell which is current. An LSP implementer following `lsp.md` will produce different completions than an architecture-conformant implementation. A tutorial author following `models.md` will teach the wrong syntax.
- **Suggested fix:** Pick one and migrate the rest in a single spec-only PR. Given the architecture spec's recency (2026-05-02) and the depth of its "Design" justification for the universal scheme, `smelt.<path>` is clearly intended to be the survivor. Update `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md` to use it. The `UndefinedModelRef` diagnostic code should likely become `UnknownSmeltPath` or similar (`functions.md` already uses `UnknownSmeltFn` for the function variant). Add a one-line note to `architecture.md` §"Resolution" pinning the migration date so future reviewers can see when the cutover happened.
- **Why it matters:** The universal addressing scheme is the smelt-vs-dbt differentiator that gives "models are functions are seeds are sources" its uniformity. If specs are split between schemes, implementers cannot know which to build, users will see inconsistent docs, and the framework's claimed structural elegance is fictional.

#### A.Critical 2: Duplicate, contradictory project-config specs

- **Spec(s):**
  - `smelt_yml.md` (last_reviewed 2026-05-03) — `paths: ["models"]` (single unified list); `name` required; `version` integer with default `1`; unknown top-level keys produce a *warning*; `default_materialization: view`.
  - `project_config.md` (last_reviewed 2026-05-03) — `model_paths: ["models"]` AND `seed_paths: ["seeds"]` (separate lists); `name` required; `version` integer (optional, "decorative"); unknown top-level keys *silently ignored*; `default_materialization: view`.
- **Observation:** Both files are dated the same day, both claim normative authority, and neither cites the other. `smelt_yml.md` Design section explicitly argues against `model_paths`/`seed_paths` ("One scan list, not per-kind … Earlier the config had `model_paths` and `seed_paths` as separate lists … Collapsing to a single `paths:` list aligns the config with the resolver's actual behaviour"). So `smelt_yml.md` is the newer rationale-first version, but `project_config.md` is still in the directory. They are not even labelled as "old" vs "new". `architecture.md`, `seeds.md`, `sources.md`, `smelt_yml.md` reference the unified `paths:` model; `models.md`, `lsp.md`, `python_models.md`, `project_config.md` reference `model_paths`/`seed_paths`.
- **Suggested fix:** Delete `project_config.md`, or shrink it to a redirect note. Update `models.md`, `lsp.md`, and `python_models.md` to reference `smelt.yml::paths` instead of `model_paths`. Take the cross-engine Parquet exchange paragraph from `project_config.md` (the only content not in `smelt_yml.md`) and put it in `architecture.md` §"Backend trait surface" or a new `cross_engine.md` stub.
- **Why it matters:** Two specs disagreeing on the project's entry point is a structural failure. A `/smelt:validate project_config` would today report "implementation matches one of two contradictory specs" — useless. Implementers and users alike must guess which is current.

#### A.Critical 3: The "engineer controls the planner" differentiator has no public surface

- **Spec(s):**
  - `planner_integration.md` §"Three planner levels", §"`smelt build --show-plan` CLI surface", `Constraints & Invariants 2` ("Pure rules"), `Out of scope for v1` (auto-derivation, cross-engine).
  - `architecture.md` §"`Transformation` and `ExecutionStep`" — exposes the value enums but never says they are user-extensible.
  - `README.md`, `CLAUDE.md` — both claim "engineers control planning" / "data engineers refactor specific logical plans to optimize" as a primary differentiator from dbt.
- **Observation:** The `README` and `CLAUDE.md` frame planner extensibility as one of the top-five differentiators ("Engineer controls planning … Planner is not a black box - the API will allow data engineers to refactor specific logical plans"). The spec set says nothing about how. `planner_integration.md` mentions `PlannerRule` once (in `Constraints & Invariants 2`: "Every `impl PlannerRule` must be `Send + Sync`") and `RuleContext` twice, but treats them as internal compiler infrastructure. There is no:
  - Public API spec for `PlannerRule`
  - Loading mechanism (Rust crate? Python rule? dynamic library? config-time enable?)
  - Discovery/registration mechanism (how does a user-written rule get added to `show_plan_rules()`?)
  - Stability contract for the rule API
  - Story for how a rule reads the workspace ("RuleContext registry lookups" — but what registry, with what shape?)
  - Lifecycle (when is the rule constructed? per-build? does it persist state?)
  - Future-work entry that names this as a planned spec (it isn't in `Out of scope for v1` either)
- **Suggested fix:** Either (1) add a `Known Divergences` entry to `planner_integration.md` saying "The planner rule API for user-authored rules is in scope for the project but pre-spec — `PlannerRule` is currently a private trait; making it public requires a separate `planner_api.md` spec covering loading, discovery, and stability"; or (2) write a stub `planner_api.md` capturing what's known today, even if it's just "the trait shape, the registration call, and a warning that everything else is unstable". Option 2 is preferable — it puts the differentiator on the spec map. `CLAUDE.md` mentions a `planner_rule_api_design.md` already exists in `docs/`; pull it into `docs/specs/` as a stub.
- **Why it matters:** This is the headline claim that distinguishes smelt's planning story from every "smarter dbt" competitor. Underplaying it in the specs makes the project look like another dbt clone with type checking. Implementers reading only the specs would conclude the planner is a closed pipeline of four hardcoded rules. That is not what the project promises elsewhere.

#### A.Critical 4: Cross-cutting user journey is broken at the testing↔incremental↔schema-evolution intersections

- **Spec(s):** `testing.md`, `incremental_models.md`, `schema_evolution.md`, `models.md`.
- **Observation:** Walked the canonical journey "define a typed model, run it incrementally on Databricks/Spark, evolve its schema, test it." Gaps:
  1. **Incremental models on Spark are pre-spec.** `incremental_models.md` Known Divergences: "MERGE strategy is DuckDB-only-future"; "A Spark MERGE pathway is in the plan but unbuilt." So step 2 of the journey (run incrementally on Spark) is aspirational, but no spec marks it as such at the user-facing surface level — `models.md` happily lists `incremental:` as a model-frontmatter key without noting target restrictions.
  2. **Tests always run on DuckDB.** `testing.md` `Constraints & Invariants 1` and `Known Divergences` "Spark test gap": "Spark-specific function behavior … cannot be tested with `smelt test`." So a Spark-only project cannot test its own models against the engine that runs them. This is a load-bearing limitation but is buried in Known Divergences rather than surfaced as a thesis-level "tests are DuckDB-only by design — here's why" Design entry.
  3. **Schema evolution interacts with incremental models with no defined behavior.** `incremental_models.md` Known Divergences: "Schema evolution is unspecified. A `partition_column` rename or an output schema change has no defined handling today." `schema_evolution.md` does not mention incremental models or partition columns at all. So step 3 of the journey (evolve schema) on an incremental model is a black hole.
  4. **Multi-target precedence is fragile.** `smelt_yml.md` Known Divergences: "Multi-target precedence with frontmatter `target:` … whether it should also be allowed to declare a target *not* defined in `smelt.yml::targets` is open." So a user pinning a single model to Spark while the project default is DuckDB has no spec-pinned behavior for the cross-engine case.
- **Suggested fix:** Add a `cross_cutting.md` (or extend `architecture.md` §"Constraints & Invariants") with a "User journey integrity matrix" naming the journeys the spec set claims to support and the gaps each one currently has. Specifically:
  - In `models.md` §"YAML frontmatter keys", note that `incremental:` is fully supported on DuckDB only today; cross-link `incremental_models.md` Known Divergences.
  - In `testing.md` §"Design", elevate "tests run on DuckDB" from Known Divergence to an explicit Design-section trade-off so users see it as a deliberate choice, not a temporary gap.
  - Open a new spec stub `incremental_schema_evolution.md` (or a section in `schema_evolution.md`) covering at minimum: partition-column rename, type changes on the partition column, schema changes that interact with the safety override list.
- **Why it matters:** A user reading the spec set bottom-up will believe each feature works; only by stitching together Known Divergences across specs do they discover the journey is broken at every join point. The thesis claims smelt is closer to production-ready than dbt; the fragmented coverage of cross-feature interactions undermines that claim.

#### A.Major 1: No spec covers the error model / diagnostic-code catalog as a whole

- **Spec(s):** Diagnostic codes are scattered across `types.md`, `functions.md`, `scoping.md`, `gradual_typing.md`, `planner_integration.md`, `lsp.md`, `incremental_models.md`, `schema_evolution.md`, `expansion.md`. `lsp.md` §"Diagnostic categories" comes closest to a global list but is incomplete.
- **Observation:** A first-class type checker / LSP / planner needs a stable diagnostic-code surface — orchestrators, CI gates, IDE configs all key on these codes. `types.md` lists 18 codes; `lsp.md` lists ~35; `functions.md` adds more; `planner_integration.md` adds 4. There is no single source-of-truth and no stability story (what happens when a code is renamed? deprecated? merged?).
- **Suggested fix:** Add a `diagnostics.md` spec listing every code, severity, anchor rule, stability tier (stable / experimental / internal), and the spec that owns it. Treat it the way a public type system treats an error code reference. The existing per-feature catalogues stay — `diagnostics.md` is the index. As a side benefit, this would surface duplicates: `UnknownIdentifier` (scoping.md) vs `UndeclaredColumn` (lsp.md) sound like overlapping codes but are not cross-referenced.
- **Why it matters:** Without a stable code surface, every CI integration breaks on every rename, and users cannot suppress specific diagnostics. This is one of the most boring-but-essential things a production tool ships.

#### A.Major 2: No spec covers run-state / build orchestration / observability

- **Spec(s):** `incremental_models.md` §"State ownership" says "smelt does not track watermarks, offsets, or run history for incremental models. The backend owns computational state." `cli.md` mentions `smelt status` and `smelt history`. `schema_evolution.md` mentions `.smelt/schemas/`. There is no spec covering: how the build runs (parallelism? topological-batch boundaries?), how failures are recovered (resume? retry? skip?), how runs are logged (run IDs? timestamps?), what `smelt status` actually reports.
- **Observation:** `cli.md` lists `smelt status [model]` ("Show incremental interval coverage and gaps") and `smelt history [model]` ("Show past run records") but neither has a spec covering the data model behind them. `cli.md` Known Divergences: "smelt status reads from live DB. Gap detection requires a database connection; this is not documented clearly in the command help." So the command exists, the spec mentions it exists, but the on-disk / on-DB state model is undocumented. This is the area with the most "smelt behaves like a build system" claims and the least spec coverage.
- **Suggested fix:** Add a `run_state.md` (or `build_lifecycle.md`) spec covering: run IDs, manifest format (referenced as `RunManifest` in `architecture.md` Crate table — pointer with no spec), `IntervalStore` (also in the table), `FileStore`, what `.smelt/` looks like on disk, what each subcommand reads/writes. This is the equivalent of dbt's `manifest.json` story; the gap is glaring once you look for it.
- **Why it matters:** Production users care about idempotency, resume-from-failure, run history, and observability. The spec set is silent on all four. `CLAUDE.md` says "we want to push towards production ready"; this is the area least ready to back that claim.

#### A.Major 3: No spec covers the multi-backend execution story end-to-end

- **Spec(s):** `architecture.md` §"Backend trait surface" defines `Backend` minimally. `project_config.md` §"Cross-engine data exchange" documents Parquet handoff in two paragraphs. `incremental_models.md` Known Divergences notes Spark MERGE is unbuilt. Backend selection rules live in `models.md` `target:` frontmatter and the precedence table in `smelt_yml.md`.
- **Observation:** The `README` differentiator "Multi-Backend: Automatically distribute work across engines" is one of the five top-level claims. Tracing it through the spec set:
  - `architecture.md` says backends implement a four-method trait, async-only.
  - `planner_integration.md` says L3 is supposed to do cross-engine choreography but no rule exists.
  - `project_config.md` (which we want to delete per Critical 2) says cross-engine references desugar to `read_parquet()`.
  - `incremental_models.md` says incremental on Spark is not built.
  - `testing.md` says tests are DuckDB-only.
  - `schema_evolution.md` has a backend capability matrix (good!) but only for ALTER TABLE.
  - No single spec answers: when the planner sees a model on backend A reading from a model on backend B, what are the rules? Where does the `read_parquet` substitution happen? When does it not work (e.g., the upstream model is a view)? Does cross-engine mean read-from-Spark-on-DuckDB, write-from-DuckDB-to-Spark, both? What about Databricks-specific features?
- **Suggested fix:** Either expand `architecture.md` §"Backend trait surface" into a fuller §"Multi-backend execution model" (preferred — it's already the home for cross-cutting backend invariants) or create `multi_backend.md`. Cover: backend selection rules, cross-engine reference resolution, capability negotiation, what's in scope for v1 vs deferred. The Parquet-exchange paragraph in `project_config.md` is the seed.
- **Why it matters:** Multi-backend is the second of the five differentiators in `CLAUDE.md`. Without a coherent spec, the implementation will accrete ad-hoc decisions and the differentiator becomes "DuckDB by default, Spark as a flag" — i.e., not actually a differentiator.

#### A.Major 4: `smelt.test` is declared in three places with three different commitments

- **Spec(s):**
  - `architecture.md` Known Divergences: "`smelt.test` declaration semantics are pre-spec. This spec introduces `smelt.test` as a top-level declaration kind so tests can live alongside models, but defers the assertion semantics … to a future `tests.md` feature spec."
  - `functions.md` §"File structure" item 4: `smelt.test` is a top-level item alongside `smelt.define` / `smelt.extern` / bare model SELECT.
  - `models.md` §"Materialization modes": `test` is a *materialization*, configured via `materialization: test` and a `test:` frontmatter object.
  - `testing.md` Surface: documents the `materialization: test` form *only*. No mention of `smelt.test` as a top-level declaration.
- **Observation:** Two distinct designs are coexisting. `architecture.md` says `smelt.test <name>` is the declaration syntax; `models.md` and `testing.md` say `materialization: test` on a model is the syntax. These are not the same thing — one is a peer of `smelt.define`, the other is an attribute of a regular model. `functions.md` references the `smelt.test` form; `testing.md` (which is the actual spec for tests) ignores it.
- **Suggested fix:** Pick one. `materialization: test` is the implemented form (per `testing.md` References pointing at `crates/smelt-core/src/metadata.rs::TestConfig`); `smelt.test <name>` appears to be aspirational. Either upgrade `testing.md` to document both forms (with a clear "current vs future" framing) or remove `smelt.test` references from `architecture.md` and `functions.md`. Whichever direction, `architecture.md` Known Divergences entry should match `testing.md` Surface — currently they describe different worlds.
- **Why it matters:** Tests are the one feature most users will encounter on day one. The spec set's inability to commit to a single test-declaration syntax — across three load-bearing specs — is a vivid example of the broader coherence problem.

#### A.Minor 1: "Strict by default" vs "lenient unknown keys" is unprincipled

- **Spec(s):**
  - `models.md` §"YAML frontmatter keys": "Unknown keys are a **hard error** (`deny_unknown_fields`)" — typos surface immediately.
  - `smelt_yml.md` §"Unknown keys": "An unknown top-level key produces a warning, not an error." — Design rationale: forward-compat staged rollouts.
  - `project_config.md` (the duplicate) §"Config loading": "An unknown top-level key does **not** cause an error" — silent.
  - `types.md` §"Strict-by-default doctrine": "settled doctrine, not a configurable mode."
- **Observation:** Three different rules for "unknown thing in YAML" depending on which file. Model frontmatter rejects; `smelt.yml` warns; `project_config.md`'s version is silent; types are strict. The Design rationale in `smelt_yml.md` is reasonable in isolation ("project-level file changes infrequently, lenient parsing helps forward compat"), but it isn't unified across the spec set, and `project_config.md`'s "silent" version contradicts even that.
- **Suggested fix:** State the doctrine once, somewhere central. Either `architecture.md` Constraints (preferred — this is genuinely a system-level invariant) or a new `error_handling.md` mini-spec. Recommended doctrine: "user-authored content (model frontmatter, type annotations) is strict; project-level config is lenient with warnings." Then have each feature spec reference the doctrine instead of restating its own version.
- **Why it matters:** Mixed strictness without a reasoned doctrine looks like accidents.

#### A.Minor 2: `gradual_typing.md` and `types.md` have a clean split — keep them this way

- **Spec(s):** `types.md`, `gradual_typing.md`, `functions.md`, `scoping.md`.
- **Observation:** This is a *good* example of the spec set working. `types.md` owns type vocabulary and bidirectional checking. `gradual_typing.md` owns tier dispatch. `functions.md` owns declaration grammar. `scoping.md` owns name resolution. Each has a "Scope" callout naming what lives where, and the cross-references resolve. Calling it out as a positive: this is what "every feature spec sits on top of architecture.md" should look like across the rest of the set.
- **Suggested fix:** Hold this up as the model in any future "how to structure related specs" guidance. Specifically the practice of opening every spec with a "Scope" block that says explicitly which adjacent specs own which things.
- **Why it matters:** The vision-level concern is whether the spec set composes; the typing trio shows it can. The other thesis areas (planner extensibility, multi-backend, run state) need this same level of discipline.

#### A.Minor 3: README differentiators table doesn't match the five differentiators in CLAUDE.md

- **Spec(s):** `README.md` §"Key Differentiators from dbt"; `CLAUDE.md` §"Key Differentiators from dbt".
- **Observation:** `README` has 5 rows: model definition, incrementalization, type checking, cross-engine, optimization. `CLAUDE.md` has 6 numbered points: logical/physical separation, engineer controls planning, cross-model optimization, multi-backend, proper language, first-class editor support. Two non-overlapping framings. The spec set roughly tracks `CLAUDE.md`, but readers will see the shorter `README` first.
- **Suggested fix:** Reconcile the two — pick one canonical differentiator list, mirror it in the other location, and use the same five categories the spec set is organised around.
- **Why it matters:** The pitch deck shouldn't disagree with the architecture brief.

### A.Cross-cutting observations

**The strongest specs were authored or reviewed last.** `architecture.md`, `planner_integration.md`, `gradual_typing.md`, `types.md`, `functions.md`, `scoping.md`, `expansion.md` — written or substantially reworked in the last 30 days — are dense, well-scoped, and cross-reference each other carefully. `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md` — also recently dated but appear to be lifted from older docs without rework — drift back into kind-prefix addressing, lighter Design sections, and Known Divergences that don't link back to spec changes elsewhere. The spec set is *recovering* from a previous shape but the recovery isn't complete. The first concrete fix-list above is roughly equivalent to "finish the architecture.md migration on the rest of the set."

**Two parallel realities exist for several features.** Beyond the headline addressing/config issues, smaller versions show up: `models.md` lists `materialization: test` while `architecture.md` introduces `smelt.test` as a top-level declaration; `models.md` shows `smelt.models.<name>(filter => …)` parameterised models while `architecture.md` says these are equivalent to `smelt.define`d functions called with explicit args. Both halves of each pair could be consistent if framed correctly, but the framing is missing from at least one side.

**Production-ready story has visible holes.** Going back to `CLAUDE.md`'s "we want to push towards production ready from a feature perspective": the five major findings above (error model, run state, multi-backend, journey integrity, planner extensibility) are exactly the spec gaps that block production confidence. The implementation may be further along than the specs suggest — but a reviewer of the spec set has no way to tell.

**Universal addressing (when it works) is genuinely a strong piece of design.** The `smelt.<path>` rule, the kind-by-content tiebreaker, the "directory layout is user-chosen" decision, the externs-are-flat exception — these are well-justified and clearly thought through in `architecture.md`. The Critical 1 finding is not "this is wrong"; it is "this is right and the rest of the spec set hasn't caught up." Fixing it pays back disproportionately because it lifts the whole spec set into self-consistency.

**Cross-references are mostly good but inconsistent in granularity.** Some specs link to specific section headings (`architecture.md` §"Resolution"); others link to a spec without a section anchor. When a section is renamed, the unanchored references silently break. A small editorial pass to upgrade all cross-spec references to anchored form would prevent silent rot. (This is also the kind of thing a future `/smelt:validate` cross-cutting check could enforce.)

---

## Appendix B: Cross-spec consistency & structural raw report

> Reviewer: consistency-lens subagent. Findings: 3 Critical, 7 Major, 7 Minor, 2 Nit. Synthesis findings cite back to it via `[C]` tags.

**Specs reviewed:** 22 (all `docs/specs/*.md`, including `SPEC_TEMPLATE.md` for reference).

### B.Summary

- **One critical contradiction.** Two specs (`project_config.md` and `smelt_yml.md`) describe the same `smelt.yml` file with **incompatible schemas** — `model_paths` + `seed_paths` vs unified `paths:`. Implementations following one will reject configs validated by the other.
- **One critical addressing-scheme split.** `architecture.md` mandates universal `smelt.<path>` addressing as a normative rule and reframes the model surface around it. Six other specs (`models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md`) still use `smelt.models.<name>` / `smelt.sources.<schema>.<table>` exclusively. The terminology has not propagated.
- **Stale "future" references.** Three specs (`gradual_typing.md`, `planner_integration.md`, `scoping.md`) flag `expansion.md` as "when written" / "planned" — but `expansion.md` exists. Two specs flag `tests.md` as future (`seeds.md`, `sources.md`, `functions.md`) — but `testing.md` exists and covers the surface. `architecture.md` defers `smelt.test` semantics to "a future `tests.md`" while `testing.md` already specifies them.
- **Template conformance is excellent.** All 22 specs have all six mandated sections (Surface, Semantics, Design, Constraints, Known Divergences, References). Two specs are missing a Design section (`incremental_models.md`, `types.md` — design content exists, but not under that heading).
- **Diagnostic-code catalogues are scattered.** Codes are listed in 5 different specs (`functions.md`, `gradual_typing.md`, `scoping.md`, `lsp.md`, `types.md`, `planner_integration.md`) with no clear "canonical home". Several codes appear in multiple specs with subtly different trigger descriptions.

### B.Template conformance matrix

Legend: ✓ present and substantive, ⚠ present but thin / partly absent, ✗ missing, FM = frontmatter.

| Spec | Surface | Semantics | Design | Constraints | Divergences | References | FM | Notes |
|------|---------|-----------|--------|-------------|-------------|------------|----|-------|
| `architecture.md`        | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | `status: stable` (only one). |
| `cli.md`                 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `data_catalog.md`        | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | No Tests block under References. |
| `datagen.md`             | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `expansion.md`           | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Notably comprehensive. |
| `functions.md`           | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `gradual_typing.md`      | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `incremental_models.md`  | ✓ | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | **No `## Design` heading**; design rationale absent. Oldest `last_reviewed` (2026-04-27). |
| `lsp.md`                 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | No Tests block under References. |
| `model_selection.md`     | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `models.md`              | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `planner_integration.md` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `project_config.md`      | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **Duplicates `smelt_yml.md` with conflicting schema.** |
| `python_models.md`       | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | No Tests block under References. |
| `schema_evolution.md`    | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | No Tests / Plans-history blocks. |
| `scoping.md`             | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `seeds.md`               | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| `smelt_yml.md`           | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **Duplicates `project_config.md` with conflicting schema.** |
| `sources.md`             | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Stub status acknowledged inline; sections concrete. |
| `testing.md`             | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Other specs still cite this as "future `tests.md`". |
| `types.md`               | ✓ | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | **No `## Design` heading**; design rationale absent. |
| `SPEC_TEMPLATE.md`       | — | — | — | — | — | — | — | Template; not reviewed for content. |

### B.Findings

#### B.Critical 1: Two normative specs describe `smelt.yml` with incompatible schemas

- **Spec(s):** `project_config.md` (last_reviewed 2026-05-03), `smelt_yml.md` (last_reviewed 2026-05-03)
- **Observation:** Both files claim to be the normative spec for `smelt.yml`. They disagree on load-bearing details:
  - **Path config.** `project_config.md` lines 24–25 declare `model_paths: ["models"]` and `seed_paths: ["seeds"]` as separate keys. `smelt_yml.md` line 20 declares a single unified `paths: ["models"]` and `smelt_yml.md` Design §"One scan list, not per-kind" (lines 83) explicitly retires `model_paths`/`seed_paths`.
  - **Unknown-key handling.** `project_config.md` Semantics says "An unknown top-level key does **not** cause an error" (lenient); `smelt_yml.md` Surface §"Unknown keys" says "produces a warning". Different observable behavior.
  - **Allowed `default_materialization` values.** `project_config.md` line 27 lists `table | view | ephemeral | materialized_view` (no `test`). `smelt_yml.md` line 22 lists `table | view | ephemeral | materialized_view | test`.
  - **Targets fallback.** `project_config.md` line 74 says "Unknown target types fall back to DuckDB"; `smelt_yml.md` Known Divergences omits this. `project_config.md` Known Divergences flags this as a bug to fix.
- **Suggested fix:** Decide the canonical home (the `feature: smelt_yml` slug and the unified `paths:` shape look like the more recent / forward design; `project_config.md` reads like the older code-anchored description). Delete the loser; redirect the surviving file's References to point to whichever feature specs (`models.md`, `seeds.md`, etc.) own per-feature config keys. Cross-references from other specs need to be updated to point to the survivor.
- **Why it matters:** Plans citing one spec will be reviewed against the other.

#### B.Critical 2: `smelt.<path>` universal addressing has not propagated through the spec set

- **Spec(s):** `architecture.md` mandates the unified scheme; `models.md`, `lsp.md`, `python_models.md`, `testing.md`, `model_selection.md`, `data_catalog.md` use the kind-prefixed legacy form.
- **Observation:**
  - `architecture.md` §"Resolution: `smelt.<path>` is the universal addressing scheme" (lines 69–104) and the Design rationale (lines 294–298) declare `smelt.<path>` as the universal address for *every* project-defined entity, replacing `smelt.ref(...)`, `smelt.source(...)`, `smelt.fn.*`. `functions.md` line 200 explicitly retires `smelt.ref(...)` and `smelt.source(...)`. `seeds.md`, `sources.md`, `smelt_yml.md` all use the universal form throughout.
  - Six specs still address models exclusively via `smelt.models.<name>` and sources via `smelt.sources.<schema>.<table>`:
    - `models.md` — Surface §"Reference syntax" (line 109) makes `smelt.models.<name>` the only documented form.
    - `lsp.md` — every diagnostic (`UndefinedModelRef`, `UndefinedSource`), every goto-definition row, every completion trigger uses the kind-prefix form.
    - `python_models.md` lines 25, 104, 183.
    - `testing.md` lines 101, 132, 145.
    - `model_selection.md` line 62: traversal "follows `smelt.models.<name>` and `smelt.sources.<name>` references".
    - `data_catalog.md` does not use either form explicitly but its diagnostic surface follows `lsp.md`.
  - The two-tiered `smelt.sources.<schema>.<table>` form in `lsp.md` does not even agree with the single-segment shape in `model_selection.md` ("`smelt.sources.<name>`").
- **Suggested fix:** Decide whether the universal scheme is the live target or an aspirational design. If live, run a sweep across the six specs above and replace `smelt.models.<name>` with `smelt.<path>` (or the workspace-relative path form), and replace `smelt.sources.<schema>.<table>` likewise. Update LSP diagnostic codes to be path-shaped (e.g. `UndefinedModelRef` → `UnknownSmeltPath` or the existing `UnknownSmeltFn` family in `functions.md`). If aspirational, demote the architecture rule to an open question and keep the legacy form.
- **Why it matters:** A reader of `architecture.md` and a reader of `models.md` will arrive at incompatible mental models. LSP code cited in `lsp.md` (`UndefinedModelRef`) does not match the universal-addressing rule in `architecture.md`. Implementations and user docs are split.

#### B.Critical 3: Multi-model file format described two ways

- **Spec(s):** `architecture.md` vs `models.md`
- **Observation:**
  - `architecture.md` §"Bare-model naming" (line 106): "In a file with two or more bare SELECTs, each bare SELECT **must** declare `name:` in its frontmatter." So multi-model files use **per-declaration YAML frontmatter** with a `name:` key.
  - `models.md` Surface §"File format" (lines 33–49): multi-model files use `--- name: <model_name> ---` **section delimiters** (not a YAML key inside frontmatter, but a special form of the frontmatter fence itself). Line 49 explicitly says "Each section delimiter must follow the exact form `--- name: <model_name> ---` ... Any other `--- X ---` form is a hard parse error."
  - `python_models.md` and `testing.md` use the `--- name: X ---` delimiter form, matching `models.md`. `functions.md` line 30 uses the architecture form ("All declared names within a file ... must be unique" via frontmatter, no delimiter syntax).
  - Either the section-delimiter form or per-declaration `name:` is the actual rule, but not both.
- **Suggested fix:** Decide which is normative and update the other. If the section delimiter is current, `architecture.md` §"Bare-model naming" needs replacing. If per-declaration `name:` is the model, `models.md`, `python_models.md`, `testing.md` all need updating. Note that `models.md` Known Divergences §"Duplicate model names undefined" makes only sense in the section-delimiter world; in the per-declaration-`name:` world, declarations are unique within a file by construction.
- **Why it matters:** A parser cannot satisfy both rules. Plans for multi-model files will diverge depending on which spec they cite.

#### B.Major 1: `expansion.md` exists but is referenced as planned/future

- **Spec(s):** `gradual_typing.md`, `planner_integration.md`, `scoping.md`
- **Observation:** `expansion.md` is a substantial spec (158 lines). Yet:
  - `gradual_typing.md` line 135: "**`expansion.md`** (when authored) will own the AST-level expansion mechanics".
  - `gradual_typing.md` line 212: "`docs/specs/expansion.md` *(planned)*".
  - `planner_integration.md` line 121: "**Expansion mechanics** (`expansion.md`, when written): how `ExpandTransparentFunctionCalls`...".
  - `planner_integration.md` line 209: "`docs/specs/expansion.md` — internal invariants for AST expansion (when written)".
  - `scoping.md` line 149: "...formalised in `expansion.md` when authored...".
  - `scoping.md` line 169: "see `expansion.md` (when authored) for the planned v2 fix".
- **Suggested fix:** Drop the "(when written)" / "(planned)" markers in those five locations; the spec exists.
- **Why it matters:** Readers think a spec is missing when it is not. New plans for expansion-touching work will spend time recreating what's already there.

#### B.Major 2: `tests.md` referenced as future, but `testing.md` exists

- **Spec(s):** `architecture.md`, `functions.md`, `seeds.md`, `sources.md`
- **Observation:**
  - `architecture.md` Known Divergences (line 324): "`smelt.test` declaration semantics are pre-spec. ... defers ... to a future `tests.md` feature spec."
  - `functions.md` line 21: "`smelt.test` declaration ... owned by a future `tests.md` spec".
  - `seeds.md` lines 148, 167: "Tests land in the shared YAML when the tests spec exists" / "when `tests.md` exists".
  - `sources.md` line 102: "deferred to the future `tests.md`".
  - `models.md` line 213 says `testing.md` is "forthcoming" while a related spec.
  - But `testing.md` exists and specifies test files, mock data, CTE isolation, assertion semantics, property-based tests, etc.
- **Suggested fix:** Either rename the file to `tests.md` and update cross-references, or update the four specs above to cite `testing.md`. Then resolve the deeper question: are there two test surfaces (`materialization: test` SQL files **and** `smelt.test` declarations), or is one the canonical and the other obsolete?
- **Why it matters:** The test-feature design space is currently in two places under different names with different shapes.

#### B.Major 3: `smelt-optimizer` crate does not exist

- **Spec(s):** `incremental_models.md`
- **Observation:** `incremental_models.md` References (lines 167–168, 175) cites `crates/smelt-optimizer/src/rules/incremental.rs`, `crates/smelt-optimizer/src/types.rs`, and "17 optimizer unit tests in `crates/smelt-optimizer/src/rules/incremental.rs`". The actual crate is `smelt-planner` (verified: `crates/smelt-planner/src/rules/incremental.rs` exists; no `smelt-optimizer` directory). `incremental_models.md` is the only spec that uses the `smelt-optimizer` name; `architecture.md` Surface lines 39–53 and `planner_integration.md` use `smelt-planner` consistently.
- **Suggested fix:** Replace `smelt-optimizer` with `smelt-planner` in `incremental_models.md` Semantics §"Batch safety classification" and References block.
- **Why it matters:** A reader trying to find the code paths for incremental rules cannot.

#### B.Major 4: `incremental_models.md` and `types.md` are missing the `## Design` section

- **Spec(s):** `incremental_models.md`, `types.md`
- **Observation:** `SPEC_TEMPLATE.md` lines 36–46 mandates the `## Design` section ("the rationale — *why* the spec is shaped this way"). The user's `feedback_specs_include_design.md` memory confirms specs must include design rationale. Both `incremental_models.md` and `types.md` lack a `## Design` heading. `types.md` does include some design-flavored notes inside Semantics, and `incremental_models.md` Constraints contains some rationale, but neither has a top-level Design section.
- **Suggested fix:** Add a `## Design` section to each spec, extracting the design rationale that's currently scattered in their other sections. For `types.md`, the strict-by-default, single-vocabulary, no-coercion choices are major design decisions deserving their own paragraphs. For `incremental_models.md`, the "no Jinja, logical SQL is pure" stance, the partition-DELETE+INSERT choice, and the "smelt does not own state" doctrine are all design decisions currently buried in Constraints.
- **Why it matters:** Future contributors can't tell when a constraint is structural vs revisitable.

#### B.Major 5: Unknown-key handling is inconsistent across specs

- **Spec(s):** `models.md`, `project_config.md`, `smelt_yml.md`, `functions.md`
- **Observation:** Three different policies for unknown YAML keys:
  - **Hard error.** `models.md` Surface line 62 and Constraints line 186: "Unknown keys are a **hard error** (`deny_unknown_fields`)". The Design rationale (line 176) defends strict rejection.
  - **Warning, not error.** `smelt_yml.md` Surface §"Unknown keys" line 59: "produces a warning, not an error".
  - **Lenient (no error).** `project_config.md` Semantics line 132: "An unknown top-level key does **not** cause an error".
  - **Warning at function frontmatter.** `functions.md` line 151: `FrontmatterParseError` "YAML parse failure (Error) or unknown key / malformed sub-entry (Warning)".
- **Suggested fix:** Either make the rule uniform or document the per-context rule clearly in `architecture.md` §"Unified frontmatter rule".
- **Why it matters:** Users get different error/warning behaviour for the same kind of mistake in different files; tools cannot reason about it.

#### B.Major 6: Diagnostic codes catalogued in 5 places with no canonical home

- **Spec(s):** `functions.md`, `gradual_typing.md`, `scoping.md`, `lsp.md`, `types.md`, `planner_integration.md`
- **Observation:**
  - `functions.md` Surface §"Diagnostic codes" lists 16 function-related codes.
  - `gradual_typing.md` Surface §"Diagnostic codes" "re-anchors the body-checking codes already catalogued in `functions.md` and adds none of its own", which is good.
  - `scoping.md` Surface §"Diagnostic codes" lists 8 codes including `UnknownIdentifier`, `ParameterShadowsColumn`, `CteCycle`. None of these are catalogued in `functions.md` or `lsp.md` with the same trigger description.
  - `lsp.md` Surface §"Diagnostic categories" lists ~40 codes, most overlapping with the per-feature specs.
  - `types.md` lists type-related codes, overlapping with `scoping.md` (e.g. `AmbiguousColumn` only appears here, not in scoping despite being a scoping concern).
  - `planner_integration.md` lists 4 planner codes.
- **Suggested fix:** Pick a single canonical catalogue. Other specs link to it and add only the single-line "triggered by ..." description for codes they own. Rule: each code is *owned* by exactly one spec; other specs may *reference* it but must not redefine its trigger.
- **Why it matters:** Drift is inevitable. Implementers need a single source of truth.

#### B.Major 7: `cli.md` mentions `seed_paths` and `sources.yml` (legacy)

- **Spec(s):** `cli.md`
- **Observation:** `cli.md` Semantics §"`smelt build` lifecycle" line 124: "Discover seed CSVs (under all `seed_paths`), `sources.yml`, SQL models, ...". Both `seed_paths` (vs unified `paths:`) and aggregate `sources.yml` (retired in favour of per-entity `.yml`) are legacy. `seeds.md` Known Divergences explicitly says "the aggregate `sources.yml` format is removed"; `sources.md` Constraints §6 makes the aggregate a "clear migration error".
- **Suggested fix:** Update `cli.md` Semantics §"`smelt build` lifecycle" to read "Discover seed CSVs, source YAMLs, SQL models, Python models, and `smelt.define` function files (under `paths:`)."
- **Why it matters:** The lifecycle description in `cli.md` is the canonical sequence. Bad description → bad implementation order.

#### B.Major 8: `lsp.md` cites `sources.yml` as a single file, contradicting `sources.md`

- **Spec(s):** `lsp.md` vs `sources.md`
- **Observation:** `lsp.md` references `sources.yml` (singular file) repeatedly: lines 60, 79, 107, 116, 146, 157, 171, 185, 213, 232. `sources.md` Constraints §6 (line 97): "Aggregate `sources.yml` at the project root is no longer recognised".
- **Suggested fix:** Update `lsp.md` to reference per-entity source `.yml` files (one per source).
- **Why it matters:** LSP behavior on the new per-entity layout is unspecified; users editing source YAMLs may not get the diagnostics they expect.

#### B.Major 9: `models.md` describes `model_paths`; reality is unified `paths:`

- **Spec(s):** `models.md`, `python_models.md`, `testing.md`
- **Observation:** `models.md` Surface §"File format" line 18: "discovered by recursively walking each directory listed in `model_paths` (default: `["models"]`)". Line 140 says the same. `python_models.md` line 16 and `testing.md` line 35, 130 all reference `model_paths`. `architecture.md` Resolution and `smelt_yml.md` Surface use the unified `paths:` key. `seeds.md` references `paths:`.
- **Suggested fix:** Rewrite all `model_paths` and `seed_paths` references to `paths:`.
- **Why it matters:** Per the Critical above, the canonical config schema is in dispute.

#### B.Minor 1: `architecture.md` is `status: stable`; everything else is `experimental`

- **Spec(s):** `architecture.md`
- **Observation:** `architecture.md` is the only spec marked `status: stable`. Every other spec is `status: experimental`. The user's `CLAUDE.md` notes the project is early-stage with no backward-compat constraints. A "stable" status amid 21 experimental specs is unusual; either `architecture.md` is also experimental or "stable" should be defined elsewhere (it is not in `SPEC_TEMPLATE.md`).
- **Suggested fix:** Either demote `architecture.md` to `experimental`, or define `stable` somewhere (`SPEC_TEMPLATE.md` is the natural home — list the allowed values for `status:`).
- **Why it matters:** A reader can't infer what `stable` promises.

#### B.Minor 2: `last_reviewed` dates lag behind recent edits

- **Spec(s):** `incremental_models.md` (2026-04-27, oldest), `types.md` (2026-04-28), several at `2026-04-29` (`expansion.md`, `functions.md`, `gradual_typing.md`, `planner_integration.md`, `scoping.md`)
- **Observation:** Today's date is 2026-05-03. Twelve specs were last reviewed on 2026-05-03; the addressing-scheme migration is referenced in `architecture.md` Design as "redesigned 2026-05-01". Specs from before that date may not have been reviewed in the new addressing context.
- **Suggested fix:** Bump `last_reviewed` after running each spec through the addressing-scheme audit.
- **Why it matters:** Plans use `last_reviewed` to decide whether to trust the spec. Stale dates conflate "old and unchanged" with "old and unchecked".

#### B.Minor 3: `sources.md` ↔ `seeds.md` broken cross-references

- **Spec(s):** `sources.md`, `seeds.md`
- **Observation:**
  - `seeds.md` line 18: "The shared YAML grammar lives in `sources.md` §"Source YAML shape".
  - `seeds.md` line 24: "A seed sidecar uses the source YAML shape (`sources.md` §"Source YAML shape")".
  - `sources.md` line 55: "The YAML grammar is shared with the seed sidecar (`seeds.md` §"Sidecar / source YAML shape")".
  - There is no `§"Sidecar / source YAML shape"` heading in `seeds.md`.
- **Suggested fix:** Decide which spec owns the grammar. `sources.md` Surface §"Source YAML shape" does in fact contain the canonical column-list grammar, so make that the canonical home.

#### B.Minor 4: `models.md` Reference syntax does not mention `smelt.define`-as-callable

- **Spec(s):** `models.md`
- **Observation:** `models.md` Surface §"Reference syntax" describes only `smelt.models.<name>` and `smelt.sources.<schema>.<table>` (line 122). `architecture.md` §"Models as functions" reframes models as parameterised functions callable like `smelt.<path>(...)`, and notes that bare-SELECT models support the same call surface. `models.md` does not mention this.
- **Suggested fix:** Add to `models.md` Surface a brief note that models inherit the universal `smelt.<path>` address and the parameterised-call surface from `architecture.md` and `functions.md`.

#### B.Minor 5: `lsp.md` `MalformedSource` and `SourceTypeError` not in `sources.md`

- **Spec(s):** `lsp.md`
- **Observation:** Two source-related codes (`MalformedSource`, `SourceTypeError`) appear in `lsp.md` but neither in `sources.md`. `sources.md` Surface §"LSP surface" mentions hover and goto-definition but no diagnostic codes.
- **Suggested fix:** Either move these codes to `sources.md` (with the diagnostic-code consolidation above) or add a Diagnostics subsection to `sources.md` Surface.

#### B.Minor 6: `data_catalog.md` "Tests" column promised but not normatively defined

- **Spec(s):** `data_catalog.md`
- **Observation:** `data_catalog.md` Surface §"Markdown output structure" line 67: "Columns table: `Column | Type | Nullable | Description | Tests`". The column-level `tests` map is currently undefined as a structured shape (the values are arbitrary strings — `data_catalog.md` Known Divergences §"Column tests are stored as strings, not validated"). Cross-reference with `models.md` line 103 (the `tests` key on column metadata) and `seeds.md` Open Questions §"Tests on seed columns" — neither has stable semantics.
- **Suggested fix:** Soften `data_catalog.md`'s promise, or pin it: "If the model's frontmatter declares column-level tests, they appear here as raw strings until `tests.md`/`testing.md` defines a structured form."

#### B.Minor 7: Tag case-sensitivity rule lives in wrong spec

- **Spec(s):** `model_selection.md` line 74 references `models.md` Tag merging
- **Observation:** Tag merging logic appears in `models.md` Semantics §"Tag merging" and `smelt_yml.md` Precedence §4. Both describe a union-with-dedup. `model_selection.md` correctly links. But neither `models.md` nor `smelt_yml.md` defines whether tags are case-sensitive; `model_selection.md` Constraints §5 says they are. The tag case-sensitivity rule should live with the tag-merging rule.
- **Suggested fix:** Move the case-sensitivity rule to `models.md` Semantics §"Tag merging".

#### B.Nit 1: `incremental_models.md` mixes addressing schemes

- **Spec(s):** `incremental_models.md`
- **Observation:** Line 95: "the injection is per-model (whole-query), not per-`smelt.<path>` reference inside the body" — uses universal addressing. Line 144: "The injected `WHERE` is applied to the outer model query once, not pushed into each `smelt.<path>` reference" — also universal. But the example in the YAML frontmatter (line 33) uses `FROM smelt.models.orders`. Internal inconsistency in one spec.
- **Suggested fix:** Use one form throughout per the addressing decision.

#### B.Nit 2: Spec filename slug vs cross-reference style varies

- **Spec(s):** All
- **Observation:** Some specs cross-reference siblings as `architecture.md`; others as `docs/specs/architecture.md`; one (`incremental_models.md` line 38) uses `smelt-yml.md` (hyphen) when the file is `smelt_yml.md` (underscore).
- **Suggested fix:** Pick one form (likely the bare filename `architecture.md`) and apply it consistently. Fix the `smelt-yml.md` typo.

### B.Cross-cutting observations

- **Two of 22 specs lack a Design section** (`incremental_models.md`, `types.md`). Every other spec has one. The user's memory explicitly requires Design rationale; this is a missing-but-required pattern, not a stylistic choice.
- **Six of 22 specs use the legacy `smelt.models.<name>` / `smelt.sources.<...>` addressing form**; the others use universal `smelt.<path>`. The migration is incomplete and the meta-spec (`architecture.md`) is the canary that the rest haven't followed.
- **Two of 22 specs duplicate each other on `smelt.yml`** with incompatible schemas. One must go.
- **Three "future spec" pointers are stale** (`expansion.md` / `tests.md` already exist) — five separate spec files reference them as planned. This is the single highest-leverage cleanup: drop "(when authored)" markers and update.
- **`smelt-optimizer` crate is referenced from one spec but does not exist** in the codebase (the crate is `smelt-planner`). One spec is fully out of date with the crate-rename event.
- **Diagnostic codes are scattered across at least 6 specs** with overlapping but non-identical descriptions. This is a structural problem that grows with each new code added.
- **`last_reviewed` dates correlate with content quality.** The two specs with `2026-04-27` / `2026-04-28` review dates (`incremental_models.md`, `types.md`) are the two missing a Design section; the seven 2026-04-29 specs are the function-family which is generally good but predates the addressing-scheme migration. The ten 2026-05-03 specs are the most recent batch.
- **Frontmatter is otherwise clean**: every spec has all four required keys, owners are uniform, status values are uniform with the one `stable` exception.

---

## Appendix C: Practitioner / user-surface raw report

> Reviewer: practitioner-lens subagent. Verdict: **Wait.** Findings: 3 Critical, 6 Major, 5 Minor, plus ergonomics red flags and migration observations. Synthesis findings cite back to it via `[P]` tags.

**Hypothetical project profile:** Mid-size analytics project: ~30 SQL models in `staging/` + `marts/` layout, 5 declared sources (CDC tables in DuckDB), 3 CSV seeds, two incremental fact tables (one daily revenue, one event-stream with late arrivals), one schema-evolving table where new columns get added monthly, `Decimal(18,2)` types for money, a small library of `smelt.define` helpers in `functions/`. Target = DuckDB locally, Spark in prod, with a couple of marts pinned to Spark.

### C.Summary

- **Adoption verdict: WAIT.** The architectural ideas are strong and the per-feature specs are mostly thoughtful — but I cannot bring this in front of my team yet because the *root* surface (`smelt.yml`, `smelt.<path>` addressing, `smelt build` lifecycle) is internally contradictory across specs.
- **Single biggest blocker: there are two specs for `smelt.yml`** (`smelt_yml.md` and `project_config.md`) that contradict each other on field names, required fields, unknown-key handling, and field surfaces.
- **Universal `smelt.<path>` addressing is half-landed.** `architecture.md` and `sources.md` say addressing is *uniform* — a model is `smelt.marts.customers`, not `smelt.models.customers`. But `models.md`, `lsp.md`, `python_models.md`, the docs-site quickstart, and the docs-site sources guide all use `smelt.models.<name>`.
- **Edge cases on incremental models — particularly first-run, schema-mismatch, late-arriving data, and partial-batch failure — have no normative answer.**
- **The migration story from dbt is missing entirely.** No spec mentions dbt analogues.

### C.The journey: building a project from these specs

I tried to walk through setting up the project profile above using *only* the specs.

**Step 1 — `smelt.yml`.** I open `cli.md`, which says "Load `smelt.yml` from `--project-dir`". Where is `smelt.yml` documented? `cli.md` references `smelt_yml.md`, `project_config.md`, and `docs-site/docs/reference/smelt-yml.md` in different places. I open all three. They disagree on whether the field is `paths` or `model_paths`/`seed_paths`. I check `examples/timeseries/smelt.yml` — it uses `paths:`. So `project_config.md` (which uses `model_paths`/`seed_paths`) is dead-letter. But it's marked `last_reviewed: 2026-05-03`, so I cannot tell if it's the new direction or the old. **I had to read source code (`crates/smelt-core/src/config.rs`) to resolve this.** That's a critical adoption-blocker.

**Step 2 — sources.** I open `sources.md` — clean, directly actionable, kind-by-content rule is clear. I drop `models/sources/raw/users.yml` and it resolves as `smelt.sources.raw.users`. Good. But: how do I know what to do if the source's external schema drifts? Spec says smelt doesn't validate at compile time and only fails at runtime via the backend. No "smelt verify". For a CDC-fed table this is scary — a column dropped upstream silently gives me NULLs everywhere downstream until someone reads the deploy logs. I'd want at least a documented escape hatch.

**Step 3 — seeds.** Clean. `seeds.md` is the best-written spec in the set. CSV inference rules are precise. The "Pin schema" code action is great UX. Two questions left unanswered: large-CSV behavior (the spec says "ephemeral seed size limits is open" — what about a 10M-row table seed loaded via `Backend::load_table`? memory? streaming?), and how seed types interact with downstream `Decimal(18,4)` cap when my data legitimately has `Decimal(18,6)` (the spec says it falls through to `Double`, silently — that's a sharp edge for finance data).

**Step 4 — first SQL model.** I write `models/staging/orders.sql` with `FROM smelt.sources.raw.orders`. This works per `architecture.md`. Now I write `models/marts/daily_revenue.sql` and want to reference orders. **What's the syntax?** `models.md` says `smelt.models.<name>` (so `smelt.models.orders`?). `architecture.md` says addressing is `smelt.<path>` so it's `smelt.staging.orders`. `lsp.md` repeatedly uses `smelt.models.<name>`. The docs-site quickstart uses `smelt.models.<name>`. The example workspace's models reference only `smelt.sources.*`, never another model, so I can't infer from realism. **I'd have to guess.** This is critical — it's the single most-typed thing in the workspace.

**Step 5 — incremental.** `incremental_models.md` is clear on the daily case. But: what does the *first run* look like? If I have a 5-year backlog of orders and I run `smelt build --event-time-start 2021-01-01 --event-time-end 2026-01-01`, does that DELETE+INSERT the entire range as one statement? Spec says "single query for any range" if `FullyBatchSafe`. So 5 years of daily revenue data goes in one INSERT? No memory caveats, no batch-size docs in the spec (the docs-site CLI shows `--batch-size` but `cli.md` and `incremental_models.md` don't). What if it OOMs at hour 10000? Spec says nothing about partial-failure recovery.

**Step 6 — late-arriving data.** Spec mentions per-column `data_latency` is "planned, not implemented". So today there's no answer. That's fine to say, but the spec should also say: "today, late-arriving data within the last `--event-time-start/--event-time-end` window will be re-incorporated on re-run; outside, you must manually rerun the affected window".

**Step 7 — schema evolution.** `schema_evolution.md` is dense and reads well. But `--allow-column-removal` and `--allow-full-refresh` flags are documented in `schema_evolution.md` and `docs-site/cli.md` — but **not in `cli.md`'s `smelt build` flag table**. So either `cli.md` is incomplete or these flags are run-only. The spec is ambiguous on `smelt build --allow-column-removal`. Also: `.smelt/schemas/` is the source of truth for `smelt diff` but its lifecycle (when written, what happens if I `git rm` it, format compatibility across versions) is in Known Divergences — i.e., explicitly unspecified. For a CI gate that's nervous-making.

**Step 8 — testing.** `testing.md` is reasonable. The CTE-isolation feature is genuinely cool. But: `smelt test --select` is substring match instead of selector syntax (per Known Divergences). That's an inconsistency every CI invocation will hit. And property-based tests' "cases: 0 behavior is undefined" — if I parametrize over an env var I will hit this on day 1.

**Net journey verdict:** roughly 60% smooth, 30% guess, 10% missing. That's not OK for a tool whose pitch is "less surprise than dbt".

### C.Findings

#### C.Critical 1: Two contradictory specs for `smelt.yml`

- **Specs:** `docs/specs/smelt_yml.md` and `docs/specs/project_config.md`. Both `last_reviewed: 2026-05-03`.
- **Observation:**
  - `smelt_yml.md` defines `paths: list of strings, default ["models"]` (single list).
  - `project_config.md` defines `model_paths` and `seed_paths` separately (default `["models"]` / `["seeds"]`).
  - `smelt_yml.md`: `name` required = yes (decorative); `project_config.md`: `name` required = yes (used in catalog).
  - `smelt_yml.md`: unknown top-level keys = warning. `project_config.md`: "An unknown top-level key does not cause an error" — silent.
  - The docs-site reference (`docs-site/docs/reference/smelt-yml.md`) and all examples use `paths:` — i.e., `smelt_yml.md` is the implementation, `project_config.md` is stale or speculative.
  - `models.md` and `seeds.md` reference `model_paths` (matching `project_config.md`); `cli.md`, `sources.md`, and `smelt_yml.md` reference `paths:` (matching the implementation).
- **Suggested fix:** Delete or supersede one. Pick `smelt_yml.md` since it matches docs-site and examples. Update `models.md`, `seeds.md`, and any other spec that references `model_paths`/`seed_paths`.
- **Why it matters:** This is the file every adopter touches first.

#### C.Critical 2: `smelt.<path>` addressing is documented inconsistently across specs

- **Specs:** `architecture.md` (universal `smelt.<path>` rule), `sources.md` (uses `smelt.sources.raw.users`), vs. `models.md` (uses `smelt.models.<name>`), `lsp.md` (uses `smelt.models.<name>` throughout), `python_models.md` (`smelt.models.<name>`), `testing.md` (`smelt.models.<name>` in `inputs:` keys), `model_selection.md` (mentions `smelt.models.<name>` for traversal), `data_catalog.md`, `cli.md` (mentions both forms), and the docs-site quickstart/sources guide.
- **Observation:** `architecture.md` §"Resolution" says the address is the literal directory path (so `models/staging/orders.sql` → `smelt.staging.orders` when `paths: ["models"]`, with the scan-root *stripped*). `sources.md` follows this. But `models.md` Surface section has examples like `FROM smelt.models.upstream_model` — which is *not* what `architecture.md` says happens.
- **Suggested fix:** Audit every spec for `smelt.models.X` / `smelt.sources.X` strings. Decide: (a) does the literal `models.` / `sources.` prefix appear in addresses or not? Either commit to prefix-stripped (and rewrite every spec example), or commit to keeping the literal prefix (and rewrite `architecture.md`'s strip rule). Then update the example workspaces to *demonstrate* model→model references so adopters can see the answer.
- **Why it matters:** This is THE most-typed expression in any smelt workspace.

#### C.Critical 3: `smelt build` flag table doesn't include schema-evolution flags

- **Specs:** `cli.md` Surface §"`smelt build` flags" + `schema_evolution.md` Surface §"Evolution flags".
- **Observation:** `schema_evolution.md` says `--allow-column-removal` and `--allow-full-refresh` are flags on `run`, `build`. `cli.md`'s `smelt build` flag table only lists `--verbose`, `--show-plan`, `--select`, `--exclude`, `--event-time-start`, `--event-time-end`. Where do `--allow-column-removal` etc. live? Are they common flags? Are they `smelt run`-only? Spec is silent.
- **Suggested fix:** Add a "common flags shared with `run`/`build`" subsection in `cli.md`, or duplicate the flag table for `smelt build`.
- **Why it matters:** Schema migrations are the single highest-stakes operation in a data pipeline.

#### C.Major 1: First-run, partial-failure, and out-of-order semantics for incremental models are unspecified

- **Specs:** `incremental_models.md` Semantics §"Execution model".
- **Observation:** Spec says re-running the same `[start, end)` is idempotent. But:
  - First-run on a 5-year-backlog incremental model — does the planner actually batch under `BoundedSafe` chunking? `incremental_models.md` says "Auto-sized chunks (3× context, clamped 7–90 partitions)". That implies batching, but `cli.md` says incremental DELETE+INSERT is per-window. The flow when `BoundedSafe(n)` meets `--event-time-start 2021-01-01 --event-time-end 2026-01-01` is not pinned down.
  - Mid-batch failure: spec is silent. If a 30-day chunk DELETEs 30 days then INSERT errors out at day 17, what's the database state?
  - Late-arriving data: per-column `data_latency` is documented as "planned, not implemented". No interim answer.
- **Suggested fix:** Add a Semantics §"First-run and backfill" subsection covering chunking under each batch-safety class, per-chunk transaction boundary, what happens on chunk-N failure, and explicit late-arriving-data handling.
- **Why it matters:** Incrementality is *the* feature dbt users come for.

#### C.Major 2: No diagnostic-code catalogue or stable error-code surface

- **Specs:** `lsp.md` lists ~50 diagnostic names; `functions.md` and `scoping.md` list theirs; `types.md` lists its set; `seeds.md` references `SeedError`; `incremental_models.md` doesn't catalog its safety-check codes.
- **Observation:** Diagnostic codes appear scattered. There is no central spec that says "this is the full list of codes, this is what each means, this is the stability guarantee". Adopters who want to suppress, route, or count specific diagnostics in CI have no contract.
- **Suggested fix:** A new spec, `diagnostics.md`, listing every code, severity, when it fires, and the stability promise.
- **Why it matters:** Production data teams gate CI on specific failures.

#### C.Major 3: `smelt test`'s `--select` substring-match inconsistency

- **Specs:** `model_selection.md` Known Divergences, `cli.md` Known Divergences, `testing.md` (silent on selector behaviour).
- **Observation:** Every command's `--select` accepts the documented selector grammar, except `smelt test`, where it's a plain substring match on test names. This is documented as a Known Divergence in two places — but a user who reads `model_selection.md` and assumes `smelt test --select tag:critical` works will get unexpected behaviour.
- **Suggested fix:** Either fix the implementation, or put a prominent warning in `testing.md` Surface, not buried in Known Divergences elsewhere.
- **Why it matters:** It's the kind of asymmetry that bites exactly once per project — and angrily.

#### C.Major 4: dbt migration / analogue mapping is entirely missing

- **Specs:** None mention dbt.
- **Observation:** Every adopter of smelt is migrating from dbt or considering it as an alternative. The specs don't reference dbt anywhere. Where does `dbt_project.yml` map? Where do `ref()` and `source()` go? `is_incremental()`? `--full-refresh`? `dbt seed`? `dbt test`? Generic tests like `unique` and `not_null`?
- **Suggested fix:** A new section in `architecture.md` or a dedicated `migration_from_dbt.md` spec.
- **Why it matters:** This is the entire adoption funnel. Practitioners do not read specs cold — they read them with dbt in their head.

#### C.Major 5: Tests, expansion, and incremental interactions for `smelt.define` functions are unclear

- **Specs:** `functions.md`, `expansion.md`, `incremental_models.md`, `testing.md`.
- **Observation:** If my incremental model body calls a `smelt.define` helper, and the planner injects a `WHERE event_time >= X AND event_time < Y` filter on the outer model: does the filter get pushed *into* the function body (`PushFilterIntoTransparentFunction` from `planner_integration.md`)? `incremental_models.md` Constraints §4 says "The injected `WHERE` is applied to the outer model query once, not pushed into each `smelt.<path>` reference in the body." But function calls aren't `smelt.<path>` references — or are they? Pushdown happens only when `provenance:` is declared. If I write a function helper and forget `provenance:`, I get `MissingProvenancePushdownAdvisory` — but is the *correctness* guaranteed if I ignore the advisory?
- **Suggested fix:** In `incremental_models.md`, add a §"Functions inside incremental bodies" that explicitly states the safety-vs-optimisation distinction.
- **Why it matters:** Function-using helpers are the killer ergonomic upgrade over dbt macros — but only if the user can be confident they're correct.

#### C.Major 6: Type system + backend interaction is mostly hand-wavy on user-visible quirks

- **Specs:** `types.md`, with backend-divergence appendix in `docs/type_semantics.md`.
- **Observation:** `types.md` claims "engine-alias normalisation" makes `Text` vs `Varchar` invisible. It also claims `Decimal(p,s)` arithmetic is "deferred — see Known Divergences". So if I run `SELECT a + b FROM x` on DuckDB where both are `Decimal(18,2)`, what's the type? Per spec it's `Decimal(38,10)` (the v1 fallback). What does DuckDB actually do? It produces `Decimal(19,2)` natively.
- **Suggested fix:** Either finalise Decimal precision arithmetic in `types.md` and remove the divergence, or very prominently document the v1 fallback in user docs with examples.
- **Why it matters:** Adopters who care about Decimal precision have no margin for surprise.

#### C.Major 7: No central spec for selectors-and-graph-traversal edge cases

- **Specs:** `model_selection.md`.
- **Observation:** Several edge cases enumerated but undecided:
  - `--exclude +model_name` removing shared deps and "potentially leaving the working set in an inconsistent state" — documented but not resolved.
  - Empty selection: `--select tag:nonexistent` matches nothing and "the working set may be empty" — silent. Is there a warning?
  - Cross-`paths:` uniqueness: `architecture.md` says cross-path collisions are workspace-load errors. `model_selection.md` doesn't say what happens during selector resolution.
- **Suggested fix:** Pin "empty working set" → warning to stderr, exit 0. Pin "selector excludes a model needed by another selected model" → either error or implicit-include with warning.
- **Why it matters:** CI scripts that run subsets break silently when these aren't pinned.

#### C.Minor 1: `smelt.yml`'s `unstable_schema:` discoverability

- **Specs:** `smelt_yml.md`, `functions.md`, `planner_integration.md`.
- **Observation:** `unstable_schema: true` gates `joins:` and `provenance:` keys. There's no `smelt unstable list` or any way to enumerate currently-gated features.
- **Suggested fix:** A `smelt unstable list` command, or a centralised list in `smelt_yml.md`.

#### C.Minor 2: `smelt docs path` is a no-op

- **Specs:** `cli.md`, `data_catalog.md`.
- **Observation:** `smelt docs path` "prints a message indicating that docs are embedded in the binary." Documented as "stub" in known divergences. Why ship a no-op command?

#### C.Minor 3: Test models' interaction with `smelt explain`

- **Specs:** `testing.md` Design, `cli.md` (`smelt explain`).
- **Observation:** `testing.md` says "test models appear in `smelt explain` output and must be explicitly excluded from execution runs". But how? Via `--exclude tag:test`? Via a special selector? Tests don't have an automatic tag.

#### C.Minor 4: `smelt seed`'s ephemeral seed size limit

- **Specs:** `seeds.md` Known Divergences.
- **Observation:** Documented: "100k-row CSV declared `materialization: ephemeral` would generate a `VALUES` literal of dangerous size. A future row-count threshold (warn, then error) is open; today's spec leaves the choice to the user."

#### C.Minor 5: `seeds.md` strict-CSV defaults are non-negotiable

- **Specs:** `seeds.md` Surface §"CSV format the loader accepts".
- **Observation:** No per-seed override in v1 (no custom delimiter, no NULL marker, no quote char). Reasonable design choice but many real-world CSVs have `|` delimiters or `\N` for NULL.

### C.Ergonomics red flags

- **Specs vs. user docs alignment is asymmetric.** Several specs cross-reference docs-site pages, but several docs-site pages aren't referenced by any spec. The `CLAUDE.md` spec rule says "if anything in Surface is user-visible, you must update the corresponding docs-site/ page in the same PR" — yet `seeds.md`, `incremental_models.md`, `schema_evolution.md`, and others have explicit "user docs out of date" entries in Known Divergences. The forcing function is failing.
- **`--show-plan` requires a positional model-file argument.** Users will type `smelt build --show-plan` (no positional) and get a hard error.
- **`smelt build --dry-run` doesn't exist; `smelt run --dry-run` does.** Asymmetric.
- **`columns:` frontmatter is split across `models.md`, `schema_evolution.md`, `data_catalog.md`, and `testing.md`.** No spec is canonical for the *full* `columns:` shape.
- **Multi-model SQL files use `--- name: <name> ---`, but the rule "lone-anonymous OR all-named" is in `architecture.md` not `models.md`.**
- **`PASSING` clause is a context-sensitive keyword.** Defensible but adopters will be confused.
- **Compile-time vs. runtime CSV inference can diverge.** `seeds.md` admits compile-time samples 100 rows, runtime reads all rows.
- **Tests always run on DuckDB regardless of project target.** `testing.md` documents this clearly but the consequence is significant.
- **`@model` decorator in Python files is great, but `@model()` (called form) "behaviour when arguments are passed... is undefined".**
- **`smelt.yml` parser is intentionally lenient on unknown keys but model frontmatter is strict.** The asymmetry will confuse users.
- **Default materialization is `view` not `table`.** Sensible but should be visible-by-default in any "set up your project" doc.

### C.Migration-from-dbt observations

- **No spec mentions dbt anywhere.** Search confirms this.
- The biggest mental leaps for a dbt user are:
  - **Refs are `smelt.<path>` not `ref('name')`.** Address by directory path (or maybe leaf name?) — but this contradicts itself across specs.
  - **No `is_incremental()`.** dbt users coming from `is_incremental()` will need to be told: "the framework injects WHERE for you; you write the same SQL for full and incremental."
  - **No Jinja.** Users will reach for `{% if execution.target == 'prod' %}` and find nothing.
  - **No exposures, no sources freshness, no docs blocks.**
  - **Tests are first-class as `materialization: test` rather than separate test SQL.**
  - **Seeds become refs but the addressing scheme is the universal `smelt.<path>` not a separate kind.**
  - **Multi-target via `target:` frontmatter and cross-engine via Parquet exchange.** This is clever but it's a totally novel pattern.
- A "dbt comparison" table in `architecture.md` would close 60% of this gap.

### C.Cross-cutting observations

- **The specs are well-structured.** Surface / Semantics / Design / Constraints / Known Divergences is a good template; consistently followed.
- **The Design sections are unusually rich and load-bearing — keep them.**
- **Specs cross-reference each other heavily.** Mostly correct, but the cross-references hide where things are *defined*. E.g., column metadata is referenced from 4+ specs but never authoritatively pinned anywhere.
- **The `last_reviewed:` dates suggest a recent (2026-05-03) push.** Several specs have that date — and some still contradict each other on that date.
- **Examples in `examples/` are realistic but limited in coverage.** `examples/timeseries/` doesn't reference one model from another (only sources). That hides the most-asked question.
- **Several specs include "Implementation lags spec" notes.** Honest, but a user evaluating today needs a single "what's actually working today" matrix.
- **No spec on observability / logging / structured output.**
- **Cross-engine data exchange is documented as a one-paragraph note in `project_config.md`.** Deserves a mention in `architecture.md` and probably its own short spec.
