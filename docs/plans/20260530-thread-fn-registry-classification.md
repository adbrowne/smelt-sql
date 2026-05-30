# Thread the function registry into the explain/classification bound derivation

**Spec:** `docs/specs/incremental_models.md`
**Spec diff:** Known Divergences — the entry "Bound derivation on the planner/classification path runs on the outer SQL body, not the expanded CST" is narrowed: the `smelt explain --json` / batch-safety classification path now derives from the **expanded** SQL (function bodies inlined), matching the execution path. The residual shrinks to the planner library itself remaining pure (it still receives whatever SQL the caller hands it).

## Problem

At execution the run pipeline expands `smelt.define` bodies before deriving source bounds (`SqlCompiler::expand_function_calls`, committed in `172d85d2`). The **classification/explain** path does not: `build_explain_output` → `compute_batch_safety_label` builds a `ModelInfo` from the raw outer `model_file.content` and calls `analyze_batch_safety`, which strips frontmatter and reads the outer SQL only. A model whose only lookback lives inside a function body (a `RANGE BETWEEN INTERVAL` in a `smelt.define` body, no outer Form B) therefore classifies as `fully_batch_safe` in `smelt explain` even though it executes with the correct widened read.

The planner (`smelt-planner`) is a pure library and depends on neither `smelt-runtime` nor `smelt-db`, so expansion must happen in the CLI layer (which has both) before the `ModelInfo` reaches the planner.

## Scope

In scope: the `build_explain_output` classification path (`smelt explain --json` `batch_safety`, and the standing `web_analytics_incremental_classification` gate).

Also folded in (Phase 4): the run/UI chunk-sizing call sites — `compute_batches_for_model` (the `smelt run --start/--end` path, expanded at the run.rs caller) and `smelt-runtime/execute.rs` (the UI path, which now builds the `FnBodyMap` up-front and expands each model before `analyze_batch_safety`). Still on outer SQL (benign, no real model affected): the bound-`NotDerivable` refusal gate (`derive_model_source_bounds`, pure planner) and the `smelt backbuild` range-expansion command.

## Phases

### Phase 1 — `smelt-runtime`: free `expand_function_calls(sql, &FnBodyMap)`
Add a standalone `pub fn expand_function_calls(sql: &str, fn_bodies: &FnBodyMap) -> String` (mirrors `resolve_refs_in_sql`: default DuckDB dialect, `smelt_path_ref: None`, fn/path-call expanders built from the map). Refactor `SqlCompiler::expand_function_calls` to delegate to it. Dialect is irrelevant here — the output feeds the text-based bound deriver, and `RANGE BETWEEN INTERVAL` round-trips identically across dialects.

- **TDD:** unit test — `expand_function_calls("… smelt.functions.f(src => smelt.x) …", {f: "… RANGE BETWEEN INTERVAL '1 day' PRECEDING … FROM src"})` inlines the body and the `RANGE` survives, while `smelt.x` stays a `smelt.<path>` ref.
- **Commit:** `refactor(runtime): extract free expand_function_calls(sql, fn_bodies)`

### Phase 2 — `smelt-cli`: expand before classification
- `build_explain_output(graph, fn_bodies: &FnBodyMap)` (new param). `compute_batch_safety_label` expands `model_file.content` via the free function before building `ModelInfo`.
- Update callers: `commands/explain.rs` builds `fn_bodies` via `build_fn_body_map(&db, ws)`; the classification test builds it from its `db`; `explain.rs` unit tests pass `&FnBodyMap::new()`.
- **TDD (red-green):** a `build_explain_output` test with a synthetic graph + a `fn_bodies` map whose function body carries `RANGE BETWEEN INTERVAL '1 day' PRECEDING` and a model with no outer Form B → classifies `bounded_safe` (was `fully_batch_safe`). Control: empty `fn_bodies` → `fully_batch_safe`.
- **Commit:** `fix(cli): classify incremental batch safety on expanded SQL (explain path)`

### Phase 3 — docs
Update `docs/specs/incremental_models.md` Known Divergence to reflect the explain path now expands; note the run/execute batching residual. Update the `web_analytics_incremental_classification` test comment if needed.
- **Commit:** folded into Phase 2.

## Progress
- Phase 1: done — free `expand_function_calls(sql, &FnBodyMap)`; `SqlCompiler` method delegates.
- Phase 2: done — `build_explain_output` takes `&FnBodyMap` and expands per model before classification + bound derivation; callers updated. Surfaced a second gap: the batch-safety temporal analyzer only inspected the outer SELECT, so an inlined function body (a derived table) was invisible; added a text scan for `RANGE BETWEEN INTERVAL` frames over the whole statement (only adds *bounded* lookback, never unbounded). Red-green test `test_batch_safety_uses_expanded_function_body`.
- Phase 3: done — spec Known Divergence narrowed.
- Phase 4: done — run/UI chunk-sizing call sites (`compute_batches_for_model` via the run.rs caller; `smelt-runtime/execute.rs` builds the registry up-front and expands per model). Divergence now covers only the refusal gate + `smelt backbuild`.

## Remaining outer-SQL call sites (not changed)

Two lower-traffic spots still classify/derive on the outer `model.sql`:

1. **Bound-`NotDerivable` refusal gate** — `derive_model_source_bounds`
   (`smelt-planner/src/rules/incremental.rs`), called from `commands/run.rs` and
   the backfill path. It lives in the *pure* planner crate, which depends on
   neither `smelt-runtime` nor `smelt-db`, so it cannot call
   `expand_function_calls` itself; closing it means pre-expanding in the CLI
   before the call (and the CLI building the registry earlier than it does today
   — the gate currently runs before `build_fn_body_map`).
2. **`smelt backbuild` range expansion** — `compute_backbuild_plans`
   (`smelt-cli/src/backfill.rs`) walks the DAG building upstream ranges from each
   model's `compute_incremental_windows`, on raw content.

Both are benign for every model in the repo: the only case that behaves
differently is a model whose lookback lives *only* inside a function body with
no outer Form B filter, and none exists.

## Open design question: expansion level is a pipeline contract, not an ad-hoc step

The pattern in this plan — "expand `smelt.define` bodies, then run the analysis"
— is applied at each call site individually, which is why the coverage came in
piecemeal. That ad-hoc-ness is a symptom: **the level of expansion a piece of
analysis needs is a real design decision, and different planner rules legitimately
want the CST at different levels.** There are at least three:

- **raw** — frontmatter-stripped outer SQL, as authored. A rule reasoning about
  the *user's* structure (e.g. "did the author write a window function in the
  outer body?", or surfacing diagnostics anchored to the source) wants this.
- **function-expanded** — `smelt.define` bodies inlined, `smelt.<path>` refs left
  intact (what `expand_function_calls` produces). Lookback/bound derivation and
  batch-safety classification want this — a window's lookback can be encapsulated
  in a function, and the bound is a property of the *expanded* logic.
- **fully resolved** — `smelt.<path>` rewritten to physical names, casts and
  time filters injected (the compiled SQL). Physical-plan / cost rules want this.

Today every analysis is handed whichever level its caller happened to pass, and
the planner being *pure* (no registry) hard-codes that it can only ever see what
the CST it was given already contains. A cleaner model would make expansion level
an explicit input to each planner entry point (or stage), so a rule declares the
level it consumes and the pipeline provides the CST at that level once, rather
than each call site re-deriving it. The two remaining outer-SQL spots above are
not really "bugs to patch one by one" — they are the same missing abstraction
showing through. Worth a dedicated design pass (research doc) before adding more
per-call-site expansion; see `docs/research/` for where that would live.
