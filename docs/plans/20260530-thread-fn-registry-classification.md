# Thread the function registry into the explain/classification bound derivation

**Spec:** `docs/specs/incremental_models.md`
**Spec diff:** Known Divergences — the entry "Bound derivation on the planner/classification path runs on the outer SQL body, not the expanded CST" is narrowed: the `smelt explain --json` / batch-safety classification path now derives from the **expanded** SQL (function bodies inlined), matching the execution path. The residual shrinks to the planner library itself remaining pure (it still receives whatever SQL the caller hands it).

## Problem

At execution the run pipeline expands `smelt.define` bodies before deriving source bounds (`SqlCompiler::expand_function_calls`, committed in `172d85d2`). The **classification/explain** path does not: `build_explain_output` → `compute_batch_safety_label` builds a `ModelInfo` from the raw outer `model_file.content` and calls `analyze_batch_safety`, which strips frontmatter and reads the outer SQL only. A model whose only lookback lives inside a function body (a `RANGE BETWEEN INTERVAL` in a `smelt.define` body, no outer Form B) therefore classifies as `fully_batch_safe` in `smelt explain` even though it executes with the correct widened read.

The planner (`smelt-planner`) is a pure library and depends on neither `smelt-runtime` nor `smelt-db`, so expansion must happen in the CLI layer (which has both) before the `ModelInfo` reaches the planner.

## Scope

In scope: the `build_explain_output` classification path (`smelt explain --json` `batch_safety`, and the standing `web_analytics_incremental_classification` gate).

Out of scope (documented, not changed): the run/execute batching call sites (`backfill.rs`, `commands/run.rs`, `smelt-runtime/execute.rs`) also classify on outer SQL, but execution correctness is already carried by the L1 bound-pushdown (which expands) plus, for the example, the outer Form B filter. A function-RANGE-*only* model with no outer Form B would still mis-*chunk* at execution; no such model exists in the examples. Folding those call sites in is a follow-up if a real case appears.

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
- Phase 1: pending
- Phase 2: pending
- Phase 3: pending
