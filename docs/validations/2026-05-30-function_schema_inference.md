## Drift Report: function_schema_inference

**Spec**: docs/specs/function_schema_inference.md (last_reviewed: 2026-05-28)
**Date**: 2026-05-30
**Phase**: B2 (feature-sweep)

### Automated checks
- cargo fmt — PASS (`cargo fmt --all -- --check`)
- cargo clippy — PASS (`cargo clippy --all-targets`, zero warnings)
- cargo test — PASS (full workspace green at pre-flight)
- example_diagnostics — PASS (87 passed, 0 failed, 1 ignored)
- example_workspaces (LSP) — PASS (27 passed, 0 failed)
- type_command_function_returns — PASS (4 passed, 0 failed) — parity regression for invariant 3
- type_property_tests @ PROPTEST_CASES=1000 — PASS (40 passed, 0 failed)

### Surface drift
- ✅ Scalar return `<name>(args) AS c` → one column of type `T`. Verified via `smelt type` on a Tier-3 call (`margin_tier3(revenue,cost) -> Expr<Double>` resolves a concrete `margin` column) and `type_command_function_returns.rs`.
- ✅ Struct `.*` spread / `smelt.as_struct(...)`. Documented in `docs-site/docs/reference/language.md` (12 occurrences of `smelt.functions` / `as_struct` / `.*`). Schema-layer expansion exercised by `examples/functions_demo/functions/enrich_order_with_as_struct.sql`.
- ✅ `FROM <name>(args) [AS a]` (TableExpr) contributes the function's output schema. Verified at the **schema layer** (`smelt type`) for direct-FROM, CTE, subquery, and JOIN-alias forms — every contributed column resolves to a concrete type (no `Unknown`). See Semantics drift below for the codegen-layer exception.
- ✅ `ColumnTypeUnresolved` is reserved and absent from code (`rg ColumnTypeUnresolved crates` → no matches) — matches the spec's Known Divergence ("reserved and not yet minted"). **Not drift.**

### Semantics drift
- ✅ Rule 1 (scalar returns) — covered by `type_command_function_returns.rs` + property tests.
- ✅ Rule 2 (struct returns / `.*`) — schema-layer expansion present (`functions_demo` clean under both diagnostic gates).
- ✅ Rule 3 (TableExpr in FROM) — schema-layer resolution correct for all argument kinds (model ref, source ref, nested call, CTE, derived table).
- ✅ Rule 4 (propagation through CTE / subquery / JOIN) — **schema layer**: confirmed transparent. `smelt type` over an adversarial probe resolved `via_cte → {margin}`, `via_subquery → {margin}`, `via_join → {revenue, margin}`, `via_from → {revenue, cost, margin}`, all concrete.
- ❌ **Constraint/Invariant 2 (schema-layer / codegen agreement) — VIOLATED for `source.*` over a `smelt.<path>` argument.** See BUG-009 in `docs/bug-hunt/2026-05-30-findings.md`. A `TableExpr` function whose body uses a qualified wildcard `source.*`, called with a model/source-ref argument (e.g. `FROM smelt.functions.add_margin(smelt.base)`), splices to `SELECT main.base.* …` — an over-qualified `schema.table.*` that DuckDB rejects (`Parser Error: syntax error at or near "*"`). `smelt type` reports the schema resolves cleanly, but `smelt run` emits invalid SQL. The schema layer and the codegen layer disagree, which Invariant 2 forbids. Latent because no existing test executes a `source.*`-bodied TableExpr function over a *model/source ref* through DuckDB (`functions_demo/margin_via_cte` passes a bare **CTE** arg, so `source.*`→`x.*` stays single-part and valid).
- ✅ Rule 5 (resolution requires callee in scope) — invariant-3 parity held; LSP and CLI `type` both resolve function-derived columns (gates green).
- ✅ Rule 6 (unresolved surfaced, never silent) — no silent `Unknown` observed; `ColumnTypeUnresolved` reserved per Known Divergence.

### Invariant drift
- ✅ Invariant 1 (pure-function rule) — not re-audited this pass; no change.
- ❌ Invariant 2 (schema-layer/codegen agreement) — violated (BUG-009, above).
- ✅ Invariant 3 (function discovery via shared loader) — `type_command_function_returns.rs` + LSP gate green.
- ✅ Invariant 4 (no silent `Unknown`) — upheld.

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage in `docs/specs/function_schema_inference.md` body. (Phase numbers appear only in code-comment fixtures under `examples/functions_demo/`, not in the spec or user docs.)

### Freshness
- last_reviewed: 2026-05-28
- most recent code change to referenced paths: 2026-05-30 (function_call.rs / schema.rs / function_body_check.rs) — within the current feature-sweep window.
- Verdict: effectively fresh (2 days); no spec-text staleness found. The BUG-009 codegen gap is a behavior defect, not spec staleness.

### Summary
- Drift items: 1 (invariant) — BUG-009 (Invariant 2 schema/codegen disagreement for `source.*` over a `smelt.<path>` arg).
- Recommended next step: `/smelt:plan` for the BUG-009 codegen fix in the transparent-function splice (smelt-planner), with an end-to-end `smelt run` gate over a model-ref TableExpr argument. Logged `needs-review` (shared run-pipeline / architectural) per the feature-sweep drift policy.
