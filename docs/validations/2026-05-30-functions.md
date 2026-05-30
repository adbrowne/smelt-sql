# Drift Report: functions

**Spec**: docs/specs/functions.md (last_reviewed: 2026-05-28)
**Date**: 2026-05-30
**Probe phase**: A4 (feature-sweep)

### Automated checks
- cargo fmt / clippy — PASS (from A1 run; tree unchanged)
- function suites — PASS: `smelt_fn_call_check` (3), `function_body_check` (5), `function_return_type` (3), `function_registry` (21)
- broken_function_diagnostics — 40 cases covering the diagnostic table (DuplicateFunctionDefinition, InvalidFunctionTypeRef, FunctionBodyTypeMismatch, ArgTypeMismatch, MissingArgument, UnknownSmeltFn, ExternCollidesWithBuiltin, BackendsWideningNotAllowed, ExternFragmentParamUnsupported, FunctionCallCycle, UnknownPassingParameter, ReturnTypeMismatch, FrontmatterParseError, UnstableSchemaRequired, AsStructUnsupportedBackend, ProvenanceMismatch, JoinsMismatch, …)
- type property oracle (function-call return typing flows into these) — PASS: `prop_nested_functions` @1000, `type_property_tests`/`prop_multi_model`/`prop_three_model`/`prop_join` @512, plus coercion/cte/setop/subquery/window @512.

### Surface drift
- ✅ `smelt.define` / `smelt.extern` grammar, call syntax, PASSING, as_struct, frontmatter keys, and the full diagnostic-code table are all present and gated. No surface drift found.

### Semantics drift
- ✅ Rules #1–#8, #10–#16 — covered by the 40 broken cases + unit suites + proptests.
- ❌ **Rule #9 (self-contained defaults) is unenforced.** Spec: "A default expression must not reference other parameters." `crates/smelt-db/src/queries/function_diagnostics.rs:1548` (`default_type_lookup`) resolves a default expression against an **empty** `TypeContext` (no parameters in scope) "self-contained per research §3" — but nothing *validates* the self-containment. A default like `b: Expr<Int> = a + 1` (referencing param `a`) is silently accepted: `a` resolves to `Unknown` against the empty context, no diagnostic fires. → **BUG-003**.

### Suspected-unenforced (not confirmed — flagged for deeper probing, not logged as bugs)
- Rule #4 (no nesting): no dedicated diagnostic for a `smelt.define` nested in a SELECT/CTE/body; error recovery treats `smelt.define` as a resync token. Whether a nested define produces *any* error is unverified.
- No diagnostic for **too many positional arguments** or an **unknown named argument** (`bogus => x`); the spec's diagnostic table has MissingArgument/ArgTypeMismatch but no over-supply / unknown-name code. Behavior unverified.
- These are intentionally NOT logged as findings yet — confirming requires the in-process diagnostic harness; revisit if the loop re-probes functions or in a dedicated diagnostics pass.

### Timeless-oracle drift
- ✅ No plan-phase vocabulary in the spec body (Known Divergences references "Phase 51" / "Phase 3" paired with plan links — tolerated).

### Freshness
- last_reviewed: 2026-05-28 (2 days ago) — fresh.

### Summary
- Drift items: 1 confirmed (Rule #9, unenforced — needs-review: enforcing requires a new diagnostic code + a spec diagnostic-table entry).
- Strong signal: the functions feature is mature. All 40 diagnostic cases, all unit suites, and the function-call-typing proptests pass. No clean auto-fixable code bug surfaced.
- Recommended next step: human review of BUG-003 (define a `DefaultReferencesParameter` code or document the rule as advisory). Deeper diagnostic-harness pass to confirm the three suspected-unenforced items.
