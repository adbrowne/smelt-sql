# smelt Codebase Review Report

**Date**: May 17, 2026
**Version reviewed**: 0.3.1
**Codebase**: ~172,000 LOC Rust, 16 crates, 2,864 tests
**Previous reviews**: [March 26, 2026](codebase-review-2026-03-26.md) (~55K LOC, 919 tests) · [April 9, 2026](codebase-review-2026-04-09.md) (~85K LOC, 1,504 tests)
**Review methodology**: Multi-perspective analysis from 10 professional viewpoints
**Author**: Independent review

---

## Executive Summary

Five weeks after the April review, smelt has more than doubled its codebase (85K → 172K LOC, +103%) without adding any new crate. Tests grew almost in lockstep (+90%). The work that landed is dominated by two ambitious additions to the language itself: **smelt-functions**, a typed SQL-fragment composition system (the "no Jinja macros" replacement), and a **typed meta-language** with lambdas, lists, higher-order functions, ternary, records, maps, pipes, and `generates: models` multi-model production from data. Together they take the "proper language instead of Jinja templates" pitch from a slogan to a concrete, statically-checked surface area that materially exceeds dbt's Jinja-derived patterns.

What did **not** happen in the same window is equally informative. With one major exception — the Salsa 0.16 → 0.26 upgrade (recommendation exceeded) and the LSP `lib.rs` split (4,116 → 47 lines, recommendation completed) — almost none of the April review's operational and community recommendations shipped. python.rs still has 125 `unwrap()` calls (unchanged). Spark still has zero integration tests outside the backend crate's inline `tests.rs`. There is no CODE_OF_CONDUCT, no issue templates, no PyPI release of `smelt-sdk`, no dbt migration guide, no plan/apply workflow, no PostgreSQL backend, no OpenLineage export, no orchestrator integration. The project made a deliberate bet on **language depth over operational maturity**, and the next review's job is to surface whether the language investment is paying back in a way that justifies deferring the operational items.

A subtle architectural change worth flagging: on April 27 the project switched to a **spec-driven workflow** (PR #111). Per-feature documents under `docs/specs/` are now normative; plans cite specs rather than restating behaviour; a `/smelt:validate` command produces drift reports. Future reviewers should treat `docs/specs/` as the canonical surface description, not the README or ROADMAP.

**Top 3 strengths:**
1. **Language ambition delivered**: smelt-functions and the meta-language are both real, working, statically typed, and end-to-end (parser → type system → planner → LSP → docs)
2. **Architectural follow-through**: the two structural recommendations from April (LSP `lib.rs` split, Salsa upgrade) shipped, plus a process-level upgrade (spec-first workflow)
3. **Test discipline preserved at scale**: 2,864 tests is +90% growth roughly matching code growth, with zero clippy warnings still enforced in CI

**Top 3 risks:**
1. **Operational maturity stalled while complexity doubled**: 103% more code, the same single author, and none of the April operational items addressed
2. **`unwrap()` debt grew faster than the codebase ratio held**: 1,157 → 1,785 absolute (+54%); python.rs unchanged at 125; failure-mode quality for users is not improving
3. **Spark and non-DuckDB backends frozen**: zero meaningful Spark work; PostgreSQL/Snowflake/BigQuery still absent; the multi-backend pitch is widening as a credibility gap

**Verdict shifts**: The dbt User perspective moves from "Cautious Adopt" to "Adopt (DuckDB)" — functions + meta-language is enough to displace Jinja macros for projects willing to learn a new language. The Spark Engineer perspective moves from "Cautious Evaluate" back to "Hold" — five weeks of zero Spark work signals deprioritisation. The Senior Rust Architect perspective stays at "Maturing Well" but with a stronger note on `unwrap()` debt. Most other perspectives are unchanged in direction but stronger in either confidence or concern.

---

## Progress Since Previous Review

### Recommendations Scorecard

#### Quick Wins (April recs)

| # | Recommendation | Status | Notes |
|---|---|---|---|
| 1 | CODE_OF_CONDUCT.md, issue templates, PR templates | **Not done** | `.github/` has only workflows and `CI.md`; no community templates |
| 2 | Publish `smelt-sdk` to PyPI | **Not done** | `python/smelt-sdk/pyproject.toml` still `0.1.0`; only `smelt-sql` (CLI) in `release.yml` publish job |
| 3 | dbt-to-smelt migration guide | **Not done** | No dbt-named docs under `docs-site/`; ROADMAP still lists this as a next step |
| 4 | "Good first issue" labels + onboarding | **Not done** | No new contributor-targeted docs beyond `developing/contributing.md` |
| 5 | LSP configuration examples for Neovim/Emacs/Helix | **Not done** | Only `editors/vscode/` exists; `docs-site/docs/guide/editor-setup.md` exists but no per-editor configs |

#### High Impact (April recs)

| # | Recommendation | Status | Notes |
|---|---|---|---|
| 1 | Audit `python.rs` `unwrap()` calls (125) | **Not done** | Still exactly 125 unwraps in `crates/smelt-cli/src/python.rs` |
| 2 | Add Spark integration test infrastructure | **Not done** | `smelt-backend-spark` grew 926 → 1,077 LOC but all 25 tests remain in inline `src/tests.rs`; no `tests/` dir, no Docker Compose, no CI Spark job |
| 3 | Continue `println!` → `tracing` migration | **Not meaningfully progressed** | 220 → 237 `println!`, 32 → 38 `tracing::`. No new structured-logging spans added |
| 4 | Audit 8 `unsafe` blocks in `schema_tracking.rs` | **N/A — was a false positive** | The April finding was diagnostic strings (e.g. `"unsafe type change on …"`), not actual `unsafe { … }` blocks. Project-wide there is a single real `unsafe` block: `mem::transmute<u16, SyntaxKind>` in `smelt-parser/src/syntax_kind.rs:376` (standard Rowan pattern). The 12 `grep "unsafe "` matches are dominated by `unsafe impl Send/Sync` on `SparkBackend` and diagnostic strings |
| 5 | Implement plan/apply workflow (approval gate + rollback) | **Not done** | `smelt diff` still exists for offline schema preview, no approval/rollback gate added |

#### Strategic (April recs)

| # | Recommendation | Status | Notes |
|---|---|---|---|
| 1 | Build PostgreSQL backend | **Not done** | `SqlDialect::PostgreSQL` enum exists; no backend crate |
| 2 | Implement OpenLineage export | **Not done** | Still in "Future / Exploration" |
| 3 | Orchestrator integration (Airflow / Dagster) | **Not done** | No adapter code or design doc |
| 4 | Build a package / dependency system | **Not done** | No work on reusable model libraries |
| 5 | Plan Salsa 0.18+ migration | **Done — exceeded** | Jumped to **Salsa 0.26** on April 18 (PR #107). Removed `catch_unwind` workaround; migrated to `#[salsa::tracked]` free functions and `cycle_initial` fixpoint iteration. Plan at `docs/plans/20260415-salsa-upgrade.md` |
| 6 | Attract a second maintainer | **Not done** | Still single-author per commit log |

**Summary: 2 of 16 recommendations addressed (1 done, 1 exceeded), 1 was a false positive, 13 not addressed.** The two that landed were the architectural items (Salsa upgrade and LSP `lib.rs` split). All operational, community, and backend-coverage items deferred.

### Improvements Not in Previous Recommendations

These shipped since the April review but were not explicitly recommended:

- **smelt-functions** (Phases 1–58 of `docs/plans/20260422-smelt-functions.md`, ~211 KB plan): typed SQL fragment language with `smelt.define`, `smelt.fn.*`, `smelt.extern`, `smelt.as_struct(<alias> EXCEPT …)`, `PASSING name AS (…)`, `Expr<T>`, `TableExpr<{…}>`, `AggExpr`, `WindowExpr`, `SelectItems<K, ctx>`. Three-tier type system (inline, isolated, return-verified). Bidirectional generics, struct row variables (`Struct<{..r}>`), value-level spread (`..event`). Function frontmatter: `provenance:`, `joins:`, `deterministic:`, `backends:`.
- **Typed meta-language** (Phases A–G of `docs/plans/20260509-meta-language-*.md`): `List<T>`, spread `...xs`, lambdas `fn x => body` and `fn (a, b) => body`, higher-order functions `map`/`filter`/`reduce`, parameterised reducers, pipe `|>`, ternary, records, `Map<K, V>`, config loaders `smelt.config.load_yaml/json/toml`, **`generates: models`** multi-model production from data with `ModelDef` closed record type and 4-stage cached Salsa workspace-shape pipeline.
- **LSP `lib.rs` decomposition**: 4,116 → 47 lines, split into `backend.rs`, `column_resolution.rs`, `completion.rs`, `db_helpers.rs`, `hover.rs`, `rename_lambda.rs`, `python_scan.rs`, `tests.rs` (plus `main.rs`).
- **Planner rule infrastructure**: `PlannerRule` trait + `apply_rules_to_fixed_point`, rules `ExpandTransparentFunctionCalls`, `PushFilterIntoTransparentFunction`, `EliminateUnusedLeftJoin`. `--show-plan` CLI flag.
- **Spec-driven workflow** (April 27, PR #111): 23 specs under `docs/specs/` as canonical reference; `/smelt:spec`, `/smelt:plan`, `/smelt:implement`, `/smelt:validate` commands.
- **Real-world validation feedback loops**: smelt_shop 0.3 follow-up (5 iteration findings → bug fixes); `/smelt-loop` automation that accumulates skill diffs and findings across runs.
- **Example workspace explosion**: 5 → 70+ example workspaces, mostly paired "working" + "broken_*" fixtures for meta-language diagnostics (47 `meta_*` directories alone).
- **Docs site expansion**: 13 new pages under `docs-site/docs/meta-language/` (config-loaders, config-vars, generators, hofs, index, lambdas, lists, maps, pipes, records, reducers, reference, reflection, ternary); functions guide; editor-features and editor-setup pages; refreshed concepts and index.

---

## Perspective 1: Current dbt User Considering a Switch

> "I've been running dbt for three years. My project has 200+ models, custom macros, and a test suite. What does smelt offer me?"

**What impresses:**

Two of the three biggest dbt-shaped holes have closed since April. **smelt-functions** is the missing macro replacement: typed SQL fragments (`Expr<T>`, `TableExpr<{…}>`, `AggExpr`, `WindowExpr`), bidirectional generics, declared properties (provenance, joins, cardinality, determinism, backends), and a planner that can transparently inline or push filters through function bodies. Where a dbt macro produces a string that gets type-checked at runtime in the warehouse, a smelt function produces a typed fragment that gets checked at authorship time and inlined into the logical plan. The April 9 review noted that the missing "reusable SQL pattern mechanism" was the dbt user's largest remaining gap — this is now solved at a higher level than dbt itself.

The **typed meta-language** is the dynamic-model replacement. `generates: models` lets a single file emit N models from data: read a YAML config, project a list of cohorts, produce a model per cohort, optionally union them. Lambdas, `map`/`filter`/`reduce`, pipes, ternary, records, and `Map<K, V>` all compose at the meta level with full type checking. The two killer demos (`examples/per_cohort_union/` and `examples/staging_from_sources/`) show patterns dbt users currently solve with Jinja loops or Python — now expressible in a typed surface with LSP support.

`smelt test` and `smelt diff` are unchanged from April but remain credible.

**What concerns:**

The learning curve is now significantly higher than dbt's. A user adopting smelt-functions has to understand generic type signatures, three tiers of body checking, row polymorphism, and the difference between transparent and opaque functions. The meta-language adds lambdas, higher-order functions, spread/destructuring, ternary, records, maps, and pipes — a real programming language overlaid on the SQL workflow. There is no migration guide explaining how dbt patterns map onto these primitives (the April rec for a dbt-to-smelt cheat sheet is still open). The dbt user has gained capability but also gained a substantial new surface to learn.

There is still no package ecosystem (no dbt-utils equivalent), no `dbt docs serve`-style interactive catalog, no snapshots/SCD Type 2.

**Verdict: Adopt (DuckDB)** (upgraded from Cautious Adopt)

For DuckDB-based projects, smelt now offers a strict superset of dbt Core's authoring capability *for users willing to learn a stronger type system*. The functions + meta-language combination provides what Jinja macros and dbt's `for` loops do, but statically typed. The cost is a steeper onboarding ramp; without the missing migration guide, this is a real adoption barrier.

**Recommendations:**

1. **Write the dbt-to-smelt migration guide** — fourth review in a row this has been recommended; with functions + meta-language landed it is finally writeable end-to-end
2. **Author 5–10 cookbook recipes for common dbt patterns** in functions/meta-language: surrogate key, date spine, staging layer normalisation, cohort split, slowly-changing dimensions
3. Add `smelt docs serve` or integrate docs into `smelt ui` for a browsable catalog

---

## Perspective 2: Director of Engineering Considering Adoption

> "One of my teams wants to adopt this. I need to understand the risk profile."

**What impresses:**

Execution velocity is again remarkable: 103% LOC growth and a major language extension in five weeks while preserving zero clippy warnings and growing the test base by 90%. The architectural follow-through is the new signal — the LSP `lib.rs` split and Salsa 0.16 → 0.26 jump both addressed April recommendations and demonstrate willingness to take on disruptive refactors. The spec-driven workflow change is the kind of process discipline normally seen in mature projects.

The docs-site has expanded substantially. The meta-language has 13 dedicated reference pages and dozens of paired example workspaces (working + broken fixtures named `*_broken_*` that act as diagnostic regression tests).

**What concerns:**

The risk profile has not improved meaningfully since April and on some axes has worsened. Bus factor of 1 is unchanged. None of the April community recommendations shipped: no CODE_OF_CONDUCT, no issue templates, no PR templates, no labels, no Discord/Discussions. There are no documented production deployments. There is no orchestrator integration. There is no operator-facing observability beyond run history.

More concerning: the project has doubled in size while operational items deferred. 103% more code is 103% more surface area to maintain alone. The April review noted that the impressive velocity was simultaneously the largest risk; that statement is now more true.

The Spark backend has gained 151 LOC and zero integration tests in 5 weeks. The signal is that Spark is no longer a priority. For an organisation evaluating multi-backend, this is decisive.

**Verdict: Wait with Interest** (unchanged)

Direction-of-travel signals are mixed. The language ambition and execution quality are strong arguments to keep watching. The single-author concentration, the stalled operational items, and the lack of any production-deployment signal mean this is still not a candidate for organisational adoption.

**Recommendations:**

1. **Document the support model**: even informally, a stated commitment about how long the author can sustain this pace and what would trigger a slowdown is information stakeholders need
2. **One time-boxed pilot on a DuckDB analytics workload** would now be defensible — the testing framework, schema evolution, functions, and meta-language are individually mature enough to evaluate
3. **Treat lack of community infrastructure as the leading indicator** to re-evaluate, not the absence of features

---

## Perspective 3: Current SQLMesh User Considering a Switch

> "I use SQLMesh for its virtual environments and plan/apply workflow. What does smelt do better?"

**What impresses:**

The functions + meta-language combination is now a clear advantage over SQLMesh's audit and macro story. SQLMesh has Python macros and Jinja support but nothing comparable to typed fragment composition or compile-time multi-model generation with type checking. For teams who would rather author transformations in a typed DSL than in Python or Jinja, smelt is now the more advanced authoring environment.

The LSP gap continues to widen. SQLMesh has a CLI and a UI but no LSP comparable to smelt's hover/goto/rename/references/code-actions stack — and now rename works for lambda parameters inside meta-language expressions.

**What concerns:**

SQLMesh's two killer features still don't exist in smelt. **No virtual environments**: no dev/prod schema comparison without materialising, no zero-downtime swap. **No formal plan/apply workflow**: `smelt diff` shows pending schema changes but there is no approval gate, no automated rollback, no snapshot-based rollforward. Backend coverage is unchanged: DuckDB plus a static Spark backend; SQLMesh covers BigQuery, Snowflake, Databricks, Postgres, DuckDB, Trino, Athena, ClickHouse, Redshift.

For a SQLMesh user the calculus has not changed: the authoring story in smelt is now stronger, but the operational story SQLMesh users adopted SQLMesh for is exactly the area where smelt has not invested.

**Verdict: Pass** (unchanged)

**Recommendations:**

1. The author should decide whether plan/apply is on the roadmap at all — SQLMesh users are not going to migrate without it, but it's a major undertaking; explicit deprioritisation is reasonable
2. Continue the LSP investment — this is the moat
3. Build at least one new backend (Postgres is the natural next step) before claiming multi-backend support

---

## Perspective 4: Senior Data Architect

> "I'm evaluating the system design for scalability, integration patterns, and long-term viability."

**What impresses:**

The architecture has absorbed an enormous amount of new behaviour without restructuring. The same crate layout (16 crates, unchanged) now hosts: a typed fragment language with three-tier checking, a meta-language with closures and HOFs, a planner rule engine with fixed-point iteration, and a multi-model generation pipeline cached through Salsa. That this all fits within the existing logical/physical split is a positive signal about the original architecture's design.

The **planner rule API** has gone from speculation to working code. `LogicalNode::FunctionCall` with `transparent` flag and `FunctionProperties` (provenance, joins, cardinality, determinism) is the metadata the planner needs to make safety-preserving rewrites. `EliminateUnusedLeftJoin` driven by declared cardinality is the first concrete example of "engineer-controlled planning" from the README pitch. This is the API the March review called out as the most promising long-term differentiator.

The **spec-driven workflow** is a quietly significant architectural move. With 23 specs under `docs/specs/` as the normative source, the project has a per-feature anchor that survives refactors, plan archives, and ROADMAP drift. This is the discipline that lets a single author sustain language-level work without losing coherence.

**What concerns:**

The largest architectural risk is now `smelt-db` size. It grew from 17,726 → 58,389 LOC in five weeks — a 3.3× expansion, driven mostly by meta-language type checking. The "pure functions, Salsa as thin wrapper" rule from CLAUDE.md is the architectural invariant that should keep this maintainable; whether the new type-checking code holds to that rule will determine whether `smelt-db` becomes the next file-split candidate or whether it stays coherent.

Cross-engine architecture is unchanged: Parquet exchange between DuckDB and Spark, no Substrait, no portable plan IR. The data architect's interest in this is unchanged.

OpenLineage / catalog integration is still absent. The type system tracks column provenance (now formally declared in function frontmatter), but there is no export API.

**Verdict: Wait with Strong Interest** (unchanged, stronger)

The architectural bets are paying off. The planner rule API, the typed function/meta-language, and the spec-driven workflow are the kind of investments that compound. The risks are concentration in `smelt-db` and the absence of operational integration points (lineage export, plan/apply).

**Recommendations:**

1. Apply the pure-function rule audit to the new meta-language type-checking code in `smelt-db` — verify it stays separable for the planned `smelt-check` extraction
2. Implement OpenLineage export now that function-level provenance is declared in frontmatter — most of the metadata exists
3. Begin sketching a portable plan IR (Substrait or custom) — the planner rule API is now mature enough that this becomes a useful abstraction layer for multi-backend
4. Document the cross-engine Parquet exchange in the docs-site (still only in the codebase)

---

## Perspective 5: Senior Analytics Engineer

> "I write 5-10 models a week and care about productivity. Will smelt make me faster?"

**What impresses:**

The combination of smelt-functions and the meta-language is a step change in authoring power. Common patterns that previously required copy-paste or Python models — date spines, cohort splitting, surrogate key derivation, standard metric calculations, staging layer normalisation — now express as typed functions or meta-language generators with LSP support. The `per_cohort_union` example is the kind of thing that takes hours in dbt (Jinja loops with macro calls, runtime errors when something doesn't unify) and reads as a short typed program in smelt.

LSP got tangible new capabilities: rename for lambda parameters, hover/completion/goto for `generates:` frontmatter, `ModelDef` field hints. The LSP `lib.rs` decomposition into 10 files is invisible to users but signals continued investment.

The example workspace count went from ~5 to ~70, with the new ones organised as paired working/broken fixtures. For an analytics engineer learning by example, this is genuinely useful.

**What concerns:**

The new authoring power comes with a learning surface that is no longer "SQL with extensions" — it's "SQL plus a typed DSL plus a meta-programming language." For senior engineers this is a feature; for ICs ramping up it's a real cost. The docs site has 13 meta-language pages and a functions guide, but no progressive disclosure ("start here, then this, then that") and no cookbook.

Editor support remains VSCode-only out of the box. The April recommendation for Neovim/Emacs/Helix configs has not landed.

**Verdict: Adopt (DuckDB)** (unchanged, stronger)

For senior analytics engineers on DuckDB, this is now an outright productivity advantage. The recommendation is unchanged because it was already "Adopt" — but the strength of the recommendation is higher.

**Recommendations:**

1. **Write a cookbook** — date spine, cohort analysis, type-2 SCD, running totals, surrogate keys — using functions + meta-language. The infrastructure exists; the examples are missing
2. **Ramp-up path documentation**: a "first 90 minutes" tutorial that takes a user from `smelt init` to a working multi-model generator
3. **Generic LSP configs**: copy-pasteable `init.lua`/Emacs lisp snippets in the docs-site

---

## Perspective 6: Senior Data Engineer (PySpark / Scala Spark)

> "My team runs 2,000+ Spark jobs daily on Databricks. I need reliable Spark integration."

**What impresses:**

Almost nothing has changed since April. The `smelt-backend-spark` crate grew from 926 to 1,077 LOC (+16%) — slower than overall codebase growth. There are 25 inline tests in `src/tests.rs` (up from 16), but no `tests/` directory, no Docker Compose for Spark standalone, no Databricks Connect mock, no CI Spark job, no Delta Lake support, no partition management examples, no GIL safety documentation, no streaming.

The schema evolution work that handles Spark's Parquet type change limitations was already in place at April 9. The `unsafe impl Send/Sync` on `SparkBackend` is unchanged and undocumented.

**What concerns:**

The signal from five weeks of zero substantive Spark work is that Spark is not a priority. For a Spark/Databricks team this is the deciding signal. The architectural story is "multi-backend"; the implementation story is "DuckDB only, plus a stub."

**Verdict: Hold** (downgraded from Cautious Evaluate)

The April verdict was "Cautious Evaluate" because the Spark backend had just gone from stub to functional. With no further investment in five weeks and no integration testing, that evaluation is not actionable. A team adopting smelt for Spark today would be the alpha user with no automated safety net.

**Recommendations:**

1. **State the Spark position explicitly** — is Spark a priority, a maintained backend, or a parking lot? Users need to know
2. If maintained: add a Docker Compose with Spark standalone and one end-to-end smoke test in CI — this is a 1-day task that would dramatically improve confidence
3. If parked: say so in the README and stop listing "multi-backend" without qualification

---

## Perspective 7: Data Analyst Maintaining a Small Project

> "I have 15 models that transform CSVs into dashboards using DuckDB. I want something simpler than dbt."

**What impresses:**

For the small-DuckDB use case, smelt is now an end-to-end complete tool. Pip install gets the CLI and LSP. The docs site has installation, quickstart, model authoring, sources, seeds, testing, and editor setup. The `smelt build` / `smelt test` / `smelt docs generate` loop works. Schema evolution is automatic for safe changes. The example workspaces include several DuckDB-only ones (`demo_workspace`, `smelt_shop_min`, `ecommerce`, `ephemeral_demo`).

**What concerns:**

The functions and meta-language are powerful but they raise the floor for a casual user. The README and quickstart need to make clear that a small project can use plain SQL models and ignore the entire functions/meta-language surface — otherwise the cognitive overhead looks higher than dbt for someone who just wants to transform CSVs.

`python.rs` still has 125 unwraps unchanged from March. For analysts using Python models, any unexpected failure still produces a Rust panic rather than a Python traceback.

There is still no community channel for "how do I X?" questions.

**Verdict: Adopt** (unchanged, with caveat)

The recommendation holds, but with a caveat the April review didn't need: the project must make clear that simple use cases don't need to engage with functions/meta-language. The complexity is opt-in, but documentation needs to communicate that explicitly.

**Recommendations:**

1. **Progressive disclosure in docs**: "for a small DuckDB project, here's the minimum you need to know — the advanced surface is documented separately"
2. **Fix python.rs unwraps** — same recommendation as March and April; this is the lowest-hanging user-experience improvement on the board
3. **GitHub Discussions** at a minimum — does not require ongoing moderation effort

---

## Perspective 8: Senior Rust Architect

> "I'm evaluating the system design, crate architecture, and long-term maintainability of this Rust codebase."

**What impresses:**

Two structural recommendations from April landed. **LSP `lib.rs` split** from 4,116 → 47 lines across 10 modules. **Salsa upgrade** from 0.16 jumped past the recommended 0.18 to 0.26, removing the `catch_unwind` workaround and migrating to `#[salsa::tracked]` free functions with `cycle_initial` fixpoint iteration. Both are disruptive changes done well — the Salsa upgrade in particular signals willingness to take on infrastructure-level work that doesn't ship user-visible features.

The CLI structure decomposed earlier in March holds: `main.rs` grew from 414 → 436 lines in five weeks despite 103% LOC growth elsewhere. The `commands/` directory now has 14 modules (added `diff.rs` and `docs.rs` since April).

The pure-function rule in CLAUDE.md remained in force through massive `smelt-db` expansion. The type-inference work for meta-language is built on top of the same pattern.

`unsafe` count is unchanged at 12 — and the April characterisation of "8 in `schema_tracking.rs`" turned out to be a measurement artefact: those grep hits were diagnostic strings (`"unsafe type change on …"`), not actual `unsafe { … }` blocks. The single real `unsafe` block in the codebase is `mem::transmute<u16, SyntaxKind>` in `smelt-parser/src/syntax_kind.rs:376`, the standard Rowan pattern. The remaining matches are `unsafe impl Send/Sync` on `SparkBackend` plus diagnostic strings.

**What concerns:**

`unwrap()` debt is now the clearest quality signal moving in the wrong direction: 935 (March) → 1,157 (April) → 1,785 (May). LOC ratio is roughly stable (~1.0%) but absolute count more than doubled in 8 weeks. `python.rs` remains at 125 unwraps, unchanged across two reviews. The pattern is: new code adds unwraps faster than old code gets refactored.

`smelt-db` at 58K LOC is the largest crate by a wide margin (3.3× growth) and a candidate for splitting. The pure-function discipline keeps it tractable but the next architectural decision worth making is whether to extract `smelt-check` (planned per CLAUDE.md) or restructure `smelt-db` internally first.

Snapshot test count is unchanged at 30 despite massive new SQL-generation surface from functions and meta-language. The infrastructure exists; the discipline has not extended to new code.

**Verdict: Maturing Well** (unchanged, with caveats)

The structural recommendations landed. The architecture has absorbed massive new behaviour cleanly. The two remaining concerns are quantitative: `unwrap()` debt and `smelt-db` size.

**Recommendations:**

1. **Systematic `unwrap()` reduction sprint**, starting with `python.rs` (125 calls). A single focused week could clear the highest-impact module
2. **Decide on `smelt-db` splitting**: either extract `smelt-check` per the original plan, or split `smelt-db` into `smelt-db-syntax` / `smelt-db-types` / `smelt-db-checks` modules. 58K LOC in one crate is approaching unworkable
3. **Expand snapshot test coverage** to functions and meta-language SQL emission — the patterns are now stable enough
4. **Document the Spark `unsafe impl Send/Sync` GIL reasoning** — outstanding since the PyO3 bridge landed

---

## Perspective 9: Senior Rust Developer

> "I'm considering contributing to this project. What's the code quality and contribution experience like?"

**What impresses:**

The contribution path is documented; CI remains strict (clippy `-D warnings`, fmt check). The crate boundaries are still clear: a contributor working on the parser doesn't need to understand Salsa; a contributor working on Spark doesn't need to touch the parser. The new spec-driven workflow under `docs/specs/` makes it easier for an external contributor to understand a feature's surface without reading source code.

Test count growth (1,504 → 2,864, +90%) approximately matches code growth (+103%), indicating the test-with-feature discipline holds. The new test surface is mostly diagnostic regression tests in paired working/broken example workspaces — a sound pattern for a compiler-shaped project.

**What concerns:**

The community infrastructure gap is now the longest-standing unaddressed item across reviews. No CODE_OF_CONDUCT, no issue templates, no PR templates, no labels, no Discussions, no Discord — three reviews running. For a project that has done substantial work on tooling, plan templates, and review skills, the absence of contributor-onboarding infrastructure is a notable inversion.

Error handling remains inconsistent: `anyhow::Result` in some modules, `thiserror`-based types in others, `unwrap()` in `python.rs`. The new code under `commands/` is mostly good; the older code has not been retrofitted.

There is no documented `git push` / PR convention for external contributors. CLAUDE.md says small changes go direct to main and larger work uses local branches + PRs — this is a single-author workflow, not a contributor workflow.

**Verdict: Good contribution experience** (unchanged)

The codebase is contributable; the community infrastructure around it is not contributor-friendly.

**Recommendations:**

1. **Adopt the minimum community templates**: a CODE_OF_CONDUCT.md (one-page copy of the Contributor Covenant), an issue template with sections for bug/feature/question, a PR template that references `docs/specs/`
2. **Standardise the error handling convention**: a one-page doc on when to use `anyhow` vs `thiserror` vs propagation
3. **Tag 5-10 "good first issue" tickets** drawn from the deferred operational items (LSP configs, docs cookbook entries, unwrap fixes)

---

## Perspective 10: Senior Python Developer

> "I want to understand the Python integration story. Can I extend smelt from Python?"

**What impresses:**

The PyO3 bridge for Python models and Spark continues to work and gained the meta-language `smelt.config.load_yaml/json/toml` integration — Python models and meta-language config loading share the same Python embedding context.

**What concerns:**

Nothing has changed for the Python developer since April. `smelt-sdk` is still `0.1.0` and not published to PyPI; the publish workflow in `release.yml` only handles the `smelt-sql` CLI. No `.pyi` stubs were added. The `python.rs` module is unchanged at 125 unwraps. The `ProjectContext` API still doesn't expose schemas, types, or the dependency graph.

The `python/smelt-rules-builtin/` package exists with two example built-in rules and `[project.entry-points."smelt.planner_rules"]` wiring — but with no PyPI presence, Python rule authors still need source installation.

**Verdict: Wait** (unchanged)

The PyO3 architecture is sound and unchanged. The distribution story (no PyPI publish), API surface (`ProjectContext` minimal), and bridge robustness (`python.rs` unwraps) are unchanged across three reviews.

**Recommendations:**

1. **Publish `smelt-sdk` and `smelt-rules-builtin` to PyPI** — three reviews running; this is the smallest change with the largest impact on the Python story
2. **Add `.pyi` stubs for the planner rule SDK** — about a day of work and enables IDE support
3. **Fix `python.rs` unwraps so Python tracebacks propagate** — the highest-value robustness fix in the codebase
4. **Expose schemas and types via `ProjectContext`** — Python rule authors need at least the type information the planner sees

---

## Cross-Cutting Themes

### Theme 1: Language Depth Over Operational Maturity

The dominant signal across all perspectives is that the project chose to build out the language (functions, meta-language) rather than the operational surface (Postgres backend, OpenLineage, plan/apply, Spark tests, community infrastructure, PyPI publish). This is a defensible choice for an early-stage project trying to establish a unique authoring story — the language is the differentiator. But it means every perspective except dbt User and Analytics Engineer is essentially in the same position as April, with five weeks of additional features they don't use.

### Theme 2: Architectural Recommendations Landed

Two structural recommendations from April shipped: the LSP `lib.rs` split (4,116 → 47 lines) and the Salsa upgrade (0.16 → 0.26, exceeding the recommended 0.18). Plus a process-level improvement: spec-driven workflow with `docs/specs/` as canonical. The pattern suggests the project will act on architectural feedback — but operational and community feedback has been deferred across three reviews. This is information about *what the author considers actionable*, which is itself useful for setting expectations.

### Theme 3: `smelt-db` Is Now The Dominant Crate

`smelt-db` grew 3.3× to 58,389 LOC — 34% of the entire codebase in a single crate, larger than the next two combined. The pure-function discipline from CLAUDE.md is what keeps this maintainable; the planned `smelt-check` extraction is overdue. This is the architectural decision the next review should look for.

### Theme 4: `unwrap()` Debt Is Compounding

935 → 1,157 → 1,785 across three reviews. The ratio to LOC is stable but the absolute count more than doubled in 8 weeks. `python.rs` is unchanged at 125 across the same period. The recommendation has appeared in all three reviews and not been acted on; the failure-mode quality story for end users is not improving.

### Theme 5: The Spark Backend Is Frozen

Five weeks of essentially no Spark work. No integration tests, no Delta Lake, no documentation. The Spark Engineer verdict downgrades from "Cautious Evaluate" back to "Hold." If Spark is not a priority, the project should state that — claiming multi-backend support without backing it is the credibility risk.

### Theme 6: Community Infrastructure Remains the Weakest Area (3rd Review Running)

No CODE_OF_CONDUCT, no issue templates, no PR templates, no labels, no Discussions/Discord, no external contributors. This has been the bottom-ranked area in every review. The project has built substantial internal tooling (review skills, smelt-loop automation, spec-driven workflow) but has not invested in inviting outsiders.

---

## Summary Matrix

| Perspective | April Verdict | May Verdict | Top Strength | Top Concern | Priority Recommendation |
|---|---|---|---|---|---|
| dbt User | Cautious Adopt | **Adopt (DuckDB)** | Functions + meta-language replaces Jinja macros and dynamic models | Learning surface much higher; no migration guide | Write the dbt-to-smelt migration guide + cookbook |
| Director of Engineering | Wait with Interest | **Wait with Interest** | Architectural follow-through; spec-driven workflow | Bus factor with 103% more code; operational items stalled | Document support model; pilot on DuckDB |
| SQLMesh User | Pass | **Pass** | LSP and meta-language advantage | No virtual envs; no plan/apply | Explicit decision on plan/apply roadmap |
| Senior Data Architect | Wait (stronger) | **Wait (stronger)** | Planner rule API + spec-driven workflow | `smelt-db` size; no lineage export | Plan `smelt-db` decomposition or `smelt-check` extraction |
| Senior Analytics Engineer | Adopt (DuckDB) | **Adopt (DuckDB, stronger)** | Functions + meta-language productivity | Learning surface; no cookbook | Write cookbook + ramp-up path |
| Data Engineer (Spark) | Cautious Evaluate | **Hold** | None new | Five weeks of zero Spark work | State Spark position explicitly |
| Data Analyst | Adopt | **Adopt (with caveat)** | DuckDB story is complete | Cognitive overhead from advanced features | Progressive disclosure in docs |
| Senior Rust Architect | Maturing Well | **Maturing Well (with caveats)** | LSP split + Salsa 0.26 landed | `unwrap()` debt; `smelt-db` size | `unwrap()` sprint + `smelt-db` decision |
| Senior Rust Developer | Good | **Good** | Spec-driven workflow aids contributors | Community infrastructure (3rd review) | Adopt minimum community templates |
| Senior Python Developer | Wait | **Wait** | PyO3 architecture sound | Nothing changed in 5 weeks | Publish smelt-sdk to PyPI |

---

## Prioritized Recommendations

### Quick Wins (< 1 week each)

1. **Publish `smelt-sdk` and `smelt-rules-builtin` to PyPI** — three reviews running; smallest change with largest Python-side impact
2. **Adopt minimum community templates**: CODE_OF_CONDUCT.md, issue template, PR template — half-day of work, unblocks contributor onboarding
3. **State the Spark position explicitly** — one paragraph in README clarifying whether Spark is priority/maintained/parked
4. **Document the cross-engine Parquet exchange** in the docs-site
5. **Write the dbt-to-smelt migration guide** — now finally writeable end-to-end with functions + meta-language landed

### High Impact (1-4 weeks each)

1. **Systematic `python.rs` `unwrap()` reduction** — 125 calls in one module, unchanged across 3 reviews; highest-value robustness fix on the board
2. **Spark integration test infrastructure** — Docker Compose with Spark standalone + one end-to-end smoke test in CI; 1-day work that would dramatically improve confidence
3. **`smelt-db` decomposition decision** — either extract `smelt-check` (per CLAUDE.md plan) or split `smelt-db` internally; 58K LOC in one crate is approaching unworkable
4. **Cookbook of 5–10 common patterns** in functions/meta-language: date spine, cohort split, SCD-2, surrogate keys, staging normalisation
5. **Expand snapshot test coverage** to functions and meta-language SQL emission
6. **Generic LSP configuration examples** for Neovim/Emacs/Helix in docs-site

### Strategic (1-3 months each)

1. **OpenLineage export** — function-level provenance is now declared in frontmatter; most of the metadata exists, the integration is mostly plumbing
2. **PostgreSQL backend** — most-requested backend after DuckDB; enables cloud-native deployments
3. **Plan/apply workflow with approval gate + rollback** — or explicitly defer in writing
4. **Orchestrator integration** (Dagster or Airflow adapter) — production scheduling story
5. **Attract a second maintainer** — most important non-technical investment for long-term viability; 3rd review running

---

## Appendix: Codebase Statistics

### Lines of Code by Crate

| Crate | LOC | Tests | Purpose |
|---|---:|---:|---|
| smelt-db | 58,389 | 778 | Salsa incremental queries, type inference (functions + meta-language) |
| smelt-cli | 27,081 | 358 | CLI entry point, orchestration, 14 subcommands |
| smelt-lsp | 19,976 | 270 | LSP (now decomposed across 10 modules) |
| smelt-parser | 18,206 | 400 | Rowan CST parser (extended for functions + meta-language) |
| smelt-core | 9,496 | 202 | Project discovery, config, dependency graphs, generators |
| smelt-types | 9,404 | 210 | Type definitions (meta-language types added) |
| smelt-state | 6,792 | 191 | Run manifests, interval tracking, schema tracking |
| smelt-planner | 6,951 | 121 | Planner rules + fixed-point iteration |
| smelt-parser-compat | 3,876 | 157 | Cross-dialect conformance |
| smelt-datagen | 3,156 | 27 | Test data generation |
| smelt-ui | 2,428 | 9 | Axum web dashboard |
| smelt-bench | 2,144 | 15 | Performance benchmarks |
| smelt-dialect | 1,943 | 83 | Multi-dialect SQL printer |
| smelt-backend-duckdb | 1,206 | 17 | DuckDB execution backend |
| smelt-backend-spark | 1,077 | 25 | Spark/PySpark execution backend |
| smelt-backend | 534 | 1 | Backend trait definition |
| **Total** | **172,659** | **2,864** | |

### Change from Previous Reviews

| Crate | March 26 | April 9 | May 17 | Apr→May Δ | Notes |
|---|---:|---:|---:|---:|---|
| smelt-db | 9,154 | 17,726 | 58,389 | +40,663 | Meta-language type checking expansion |
| smelt-cli | 12,320 | 18,062 | 27,081 | +9,019 | New commands, function execution wiring |
| smelt-lsp | 3,270 | 9,469 | 19,976 | +10,507 | Meta-language LSP features; `lib.rs` decomposed |
| smelt-parser | 7,956 | 9,483 | 18,206 | +8,723 | Function + meta-language syntax |
| smelt-core | 3,723 | 4,404 | 9,496 | +5,092 | Generator file infrastructure |
| smelt-types | 1,390 | 2,051 | 9,404 | +7,353 | Meta-language type system |
| smelt-planner | 1,750 | 2,938 | 6,951 | +4,013 | PlannerRule trait + rules |
| smelt-state | 0 | 6,747 | 6,792 | +45 | Essentially unchanged |
| smelt-backend-spark | 267 | 926 | 1,077 | +151 | Minimal activity |
| smelt-backend-duckdb | n/a | 616 | 1,206 | +590 | Added tests |

### Quality Metrics Comparison

| Metric | March 26 | April 9 | May 17 | Apr→May Trend |
|---|---:|---:|---:|---|
| Total Rust LOC | 55,093 | 85,072 | 172,659 | +103% (more than doubled) |
| Test functions | 919 | 1,504 | 2,864 | +90% (tracking LOC growth) |
| `unwrap()` calls | 935 | 1,157 | 1,785 | +54% (debt compounding) |
| `unwrap()` in `python.rs` | n/a | 125 | 125 | Unchanged across 3 reviews |
| `println!()` calls | 320 | 220 | 237 | Essentially flat |
| `tracing::` calls | 8 | 32 | 38 | Slow growth |
| `unsafe` blocks (real) | n/a | (April had measurement error) | 1 | The single real `unsafe`: Rowan `transmute` pattern |
| `unsafe` grep proxy | 4 | 12 | 12 | Unchanged; dominated by `impl Send/Sync` + diagnostic strings |
| Fuzz targets | 2 | 2 | 2 | Unchanged |
| CI workflows | 9 | 9 | 9 | Unchanged |
| Snapshot tests | 0 | 30 | 30 | Unchanged despite massive new SQL surface |
| CLI `main.rs` lines | 2,387 | 414 | 436 | Decomposition holds |
| LSP `lib.rs` lines | n/a | 4,116 | 47 | **Decomposed into 10 modules** |
| Clippy warnings | 0 | 0 | 0 | Enforced in CI |
| Example workspaces | 5 | 5 | ~70 | Mostly paired working/broken meta-language fixtures |

### Key Dependencies

| Dependency | Version | Change from April | Purpose |
|---|---|---|---|
| rowan | 0.15 | unchanged | Lossless CST representation |
| **salsa** | **0.26** | **0.16 → 0.26** | Incremental computation (upgraded; April rec exceeded) |
| datafusion | 43 | unchanged | SQL type coercion (validation only) |
| duckdb | 1.4.4 | unchanged | Execution backend + test oracle |
| arrow | 58 | unchanged | Data interchange |
| parquet | 58 | unchanged | Data storage |
| tower-lsp | 0.20 | unchanged | LSP protocol |
| tokio | 1 | unchanged | Async runtime |
| thiserror | 2.0 | unchanged | Structured errors |
| anyhow | 1.0 | unchanged | Error context |
| proptest | 1.4 | unchanged | Property-based testing |
| pyo3 | 0.28 (abi3-py39) | unchanged | Python embedding |
| insta | 1 | unchanged | Snapshot testing |
| clap | 4 (derive) | unchanged | CLI parsing |
| serde | 1 (derive) | unchanged | Serialization |

### Notable Workflow / Process Changes

- **Spec-driven workflow** (April 27, PR #111): 23 specs under `docs/specs/` are now normative; plans cite specs; `/smelt:spec` `/smelt:plan` `/smelt:implement` `/smelt:validate` commands. Future reviewers should treat `docs/specs/` as the canonical surface description.
- **`/smelt-loop` automation**: per-iteration findings batching with skill diffs applied automatically; visible in commit messages like `chore(meta-language-G): /smelt-loop tier-3 fixture audit`.
- **Phase-commit discipline**: each phase commits a SHA marker after the phase commit lands, providing a navigable progress record on the PR branch.
