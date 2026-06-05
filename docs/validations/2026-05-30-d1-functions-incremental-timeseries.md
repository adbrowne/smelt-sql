## Drift Report: D1 — functions × incremental × timeseries (combination phase)

**Specs**: docs/specs/functions.md, docs/specs/incremental_models.md, docs/specs/timeseries.md
**Date**: 2026-06-05
**Phase**: D1 (feature-sweep combination probe)

### Automated checks
- cargo fmt — PASS
- cargo clippy — PASS (zero warnings)
- cargo test — PASS (full workspace green)
- example_diagnostics — PASS (95 passed, 1 ignored)

### Seam: function expansion inside incremental models

The D1 probe exercised the claim in `incremental_models.md` §"Functions inside incremental bodies":
> "Function expansion is a logical pass that runs **before** every analysis stage in this spec — per-source bound derivation, batch-safety classification, and source-filter pushdown all see the expanded CST."

Verification result per pipeline path:

| Analysis stage | Expands fn bodies? | Code path | Spec claim |
|---|---|---|---|
| Batch-safety classification | ✅ YES | `run.rs:1158` `expand_function_calls` before `compute_batches_for_model` | Correct |
| Source-filter pushdown (bound derivation) | ✅ YES | `run.rs:1252` `expand_function_calls` before `build_source_bound_map` | Correct |
| Source-filter pushdown (injection) | ⚠️ PARTIAL | `run.rs:1254` `inject_source_filters(&clean_sql, ...)` — injects into **unexpanded** SQL | See below |
| NotDerivable refusal gate | ⚠️ NO | `run.rs:593` `derive_model_source_bounds(model_info, ...)` — uses raw `model.sql` | Documented in Known Divergences |
| Window function safety check (CLI) | ⚠️ NO | `run.rs:558` `planner.plan(&opt_graph)` → `detect()` → `model.sql` (unexpanded) | Implicit in Known Divergences |
| Window function safety check (LSP) | ⚠️ NO | `lib.rs:1303` `detect_builtin_rules(&ctx)` where `ctx.sql = stripped` (unexpanded) | Not explicitly listed in Known Divergences |

### Surface drift
- ✅ `smelt.define` bodies containing predicates used in incremental model WHERE clauses — function expansion at compile time works correctly; e2e test `fn_incremental_ts_e2e.rs` confirms.
- ✅ `smelt.functions.<fn_name>(...)` call paths resolve correctly — file stem is NOT a path component (confirmed: `smelt.functions.is_peak_hour` for `functions/event_helpers.sql::is_peak_hour`).
- ✅ Time filter injection (`WHERE partition_column >= start AND partition_column < end`) happens after function expansion — the injected clause sees function-produced columns.
- ✅ Source-filter pushdown works when source refs appear in outer model SQL (or as named function arguments).
- ⚠️ Source-filter pushdown gap: source refs ONLY inside a function body (not in outer SQL or as arguments) are NOT injected. Bound derivation correctly derives the bound (from expanded SQL), but `inject_source_filters` operates on unexpanded SQL. The Known Divergences section documents "Two lower-traffic spots still classify on the outer `model.sql`", but doesn't explicitly call out the injection gap for sources buried in function bodies.

### Semantics drift
- ✅ **Idempotence with function bodies (Constraint #7)**: verified by `fn_incremental_ts_e2e.rs::fn_incremental_ts_function_expansion_and_time_filter` — re-running same window after function expansion produces same output.
- ✅ **Function predicate filtering**: `is_peak_hour(event_ts)` in WHERE correctly filters out off-peak rows in incremental run (verified end-to-end).
- ✅ **SELECT function calls**: `hour_bucket(event_ts)` in SELECT produces correct bucket labels in output.

### Invariant drift
- ✅ Constraint #5 ("source-filter pushdown is per-reference"): for source refs in outer SQL, pushdown works correctly per-reference including when function call arguments reference the source.
- ⚠️ Constraint #10 ("No silent downgrade"): the `detect_builtin_rules` safety check runs on unexpanded SQL. A non-partition-aligned OVER inside a function body would NOT be detected by the incremental safety check — the model would be accepted silently. This is the window-function safety bypass gap noted in the Known Divergences section. Found via probe: the `fn_incremental_ts_broken_safety_bypass` fixture demonstrates the LSP DOES catch the pattern via the `detect_builtin_rules` (which also runs on unexpanded SQL, but the diagnostic cascade shows the LSP path is consistent with the CLI path in this regard).

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage in incremental_models.md body (one "Phase 1" reference at line 308 is inside Known Divergences with a plan link context — tolerated).
- ✅ No phase-vocabulary in functions.md body sections (two matches in Known Divergences and References sections — tolerated).
- ✅ No phase-vocabulary in timeseries.md body.

### Freshness
- last_reviewed (incremental_models.md): 2026-05-21
- Most recent relevant code change: 2026-06-05 (fn_incremental_ts probe; codegen soundness fixes)
- Verdict: fresh for D1 probe purposes; the Known Divergences section is accurate.

### D1 findings summary
- **Bugs fixed**: 0 (seam is sound for the probed patterns)
- **Needs-review**: 0 (the limitation re: source refs buried in function bodies is pre-documented)
- **Docs-gap (deferred)**: 1 — the Known Divergences in `incremental_models.md` should explicitly mention the window function safety check (`detect_builtin_rules`) as a non-expanding site alongside `derive_model_source_bounds` and `compute_backbuild_plans`.
- **New test coverage**: `crates/smelt-cli/tests/fn_incremental_ts_e2e.rs` (e2e function expansion + time filter injection in incremental run), `crates/smelt-cli/tests/example_diagnostics.rs::fn_incremental_ts_no_diagnostics` (LSP clean), `examples/fn_incremental_ts/` (probe fixture).

### Summary
- Drift items: 1 docs-gap (deferred)
- Recommended next step: none urgent. Defer the Known-Divergences clarification to the next spec freshness pass.
