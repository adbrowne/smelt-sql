# Expansion level as a planner-pipeline contract

**Status:** research / design clarity — not a committed scope. Tracked in issue #130.
**Prompted by:** `docs/plans/20260530-thread-fn-registry-classification.md` (threading the `smelt.define` registry into bound derivation / batch-safety classification) and the recurring "expand the function bodies first, then analyze" pattern that came up across L1, L8, and the classification work.
**Related:** `docs/specs/expansion.md` (function expansion), `docs/specs/incremental_models.md` (bound derivation, batch-safety, source-filter pushdown), `docs/research/20260521-incremental-as-planner-rule.md`.

## The observation

Several analyses — per-source bound derivation, batch-safety classification, the source-filter pushdown, the `NotDerivable` refusal gate — must reason about SQL that has had its `smelt.define` calls *inlined*, because a lookback (`RANGE BETWEEN INTERVAL '1 day' PRECEDING`) or a window function can be encapsulated inside a function body. We retrofitted this one call site at a time: the run pipeline expands before deriving source bounds (L1), `build_explain_output` expands before classifying, the run/UI chunk-sizing expands before `analyze_batch_safety`. Two call sites still read the outer SQL (the refusal gate in the pure planner; `smelt backbuild`).

The piecemeal rollout is the symptom. **The real question is not "which call sites did we miss" but "at what level of expansion should each analysis see the program, and whose job is it to produce that level."**

## Levels

There are (at least) three distinct forms of a model's SQL, and different analyses legitimately want different ones:

| Level | What it is | Who wants it |
|---|---|---|
| **raw** | frontmatter-stripped outer SQL, as authored | rules reasoning about the *author's* structure: source-anchored diagnostics, "did the user write a bare `OVER` in the outer body?", lint/style |
| **function-expanded** | `smelt.define` bodies inlined, `smelt.<path>` source refs left intact | lookback/bound derivation, batch-safety classification, source-filter pushdown — a lookback is a property of the *expanded* logic, and pushdown still needs the `smelt.<path>` names to target |
| **fully resolved** | `smelt.<path>` → physical table names, casts + time filters injected (the compiled SQL) | physical-plan / cost / engine-dialect rules |

`expand_function_calls` (added in this arc) produces the middle level deliberately — it sets `smelt_path_ref: None` so refs survive. That distinction (inline functions, keep refs) is exactly a *level*, and right now it exists only as an ad-hoc helper that individual callers remember to call.

## The tension that makes this a design question, not a bug list

`smelt-planner` is a **pure** crate: it depends on neither `smelt-runtime` (which owns `SqlCompiler` / `expand_function_calls`) nor `smelt-db` (which owns the function registry via Salsa). That purity is a load-bearing invariant — it's what lets bound derivation, batch-safety, and the planner rules be unit-tested without a database and lets a future `smelt-check` crate reuse them.

A pure planner therefore **cannot expand its own input** — it analyzes whatever CST string it was handed. So "what level does this analysis run at" is decided entirely by the *caller* in the CLI/runtime layer. Today that decision is implicit and inconsistent:

- `derive_model_source_bounds` (planner) is handed `Frontmatter::strip(model.sql)` — raw — and so silently under-derives for function-encapsulated lookbacks. It *cannot* fix this itself; the CLI must pre-expand before calling it.
- `analyze_batch_safety` (planner) likewise sees whatever its caller passes; we made each caller pass expanded SQL.

So the two remaining outer-SQL call sites are not isolated oversights — they are the same missing abstraction: **there is no explicit contract for "this planner entry point consumes level X," and no single place that produces level X once.**

## Sketch of cleaner models (not a decision)

Options worth weighing in a real design pass:

1. **Level as an explicit parameter / newtype.** Replace bare `&str` SQL inputs to planner entry points with a typed `Sql<Level>` (e.g. `RawSql`, `FnExpandedSql`, `ResolvedSql`). The CLI produces each level once via the runtime; the planner's signatures state which level they require, so passing raw where function-expanded is needed is a *type error* instead of a silent under-derivation. Keeps the planner pure (it receives the leveled string; it doesn't produce it).

2. **Expansion as an explicit logical pass with addressable outputs.** Make function expansion a named stage (per `expansion.md`) whose output CST is a first-class artifact the orchestrator caches and hands to downstream rules by level. Planner rules declare a required level the way they might declare other inputs.

3. **A rule-plug-in contract.** If planner rules become pluggable (cf. `docs/research/20260521-incremental-as-planner-rule.md` and the planner-rule API direction), each rule could declare the expansion level it consumes as part of its registration, and the harness materializes inputs accordingly — generalizing beyond the few hard-coded analyses we have today.

Cross-cutting questions:
- Is "function-expanded with refs intact" the only intermediate level, or are there more (e.g. metric expansion, ephemeral inlining) that deserve their own level?
- Where does expansion run for the LSP/Salsa path vs the CLI/runtime path, and can a single leveled-CST query in `smelt-db` serve both (keeping the pure-function rule)?
- Cost: expanding once per model per level vs re-expanding per call site (today's helper re-parses each time).

## Why not just "expand everywhere before the planner"

That is the tempting quick fix (and is what the per-call-site retrofits approximate), but it bakes in *function-expanded* as the universal level and would be wrong for a rule that genuinely wants the raw authored form (diagnostics that must point at the user's `smelt.define` call site, not the inlined body) or the fully-resolved form. A uniform pre-expansion also fights the pure-planner invariant by pushing expansion responsibility ambiguously between layers. The levels are real; collapsing them is the thing to avoid.

## Disposition

Keep the two remaining outer-SQL call sites as documented (benign today — no model in the repo has a function-internal-only lookback with no outer Form B). Do **not** add a third per-call-site expansion patch without first deciding the level contract above. This doc exists to make that decision deliberate rather than incremental.
