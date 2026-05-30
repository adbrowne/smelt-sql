## Drift Report: gradual_typing

**Spec**: docs/specs/gradual_typing.md (last_reviewed: 2026-05-09)
**Date**: 2026-05-30
**Phase**: B4 (feature sweep)

### Automated checks
- cargo fmt — PASS (`cargo fmt --all -- --check`)
- cargo clippy — PASS (no warnings, `--all-targets`)
- cargo test — PASS (full suite green at pre-flight; 0 failed)
- example_diagnostics — PASS (87)
- example_workspaces (LSP) — PASS (27)
- type_property_tests @ default — PASS (40)
- function_body_check unit tests — PASS (18, 2 ignored — `columns_of` HOF wiring deferred)

### Surface drift

- ✅ **Tier dispatch is implicit** — `compute_tier(params, return_type_text)` at `crates/smelt-types/src/signatures.rs:2215` derives Tier 1/2/3 from annotation completeness. Tier 1 = any unannotated param; Tier 2 = all annotated, no return; Tier 3 = all annotated + return. Covered by `tier_two_when_params_annotated_return_missing`, `tier_three_when_fully_annotated`, `extracts_minimal_signature`.
- ✅ **`TableExpr`/`SelectItems` count as annotated** — bare `TableExpr` parses to `Some(Ok(TableExpr(None)))` (`type_ref_text.is_some()` ⇒ counted). Probe confirmed `f(source: TableExpr)` → Tier 2; `f(source: TableExpr<{a: Integer}>)` → Tier 2.
- ❌ **"A malformed annotation (one that fails `InvalidFunctionTypeRef`) is treated as unannotated, demoting the function to Tier 1"** — **NOT implemented**. `compute_tier` keys on `ParamSpec::type_ref_text.is_some()`, which is `true` for malformed annotations (the raw text is present even when the parse fails). Probe (`signatures.rs`): `f(x: Bogus)`, `f(x: Expr<Bogus>)`, `f(x: FooExpr<Integer>)`, and `f(x: Expr<Struct<{a: Bogus}>>)` (all of which emit `InvalidFunctionTypeRef`) are classified **Tier 2**, not Tier 1. → **BUG-012** (needs-review).
- ✅ **Diagnostic codes** — `FunctionBodyTypeMismatch`, `ReturnTypeMismatch`, `ArgTypeMismatch`, `MissingArgument`, `FragmentColumnMissing`, `FragmentKindMismatch` all present in `crates/smelt-db/src/diagnostics_types.rs` and fired from the tier paths (`function_body_check.rs`).
- ✅ **`DiagnosticData::ExpansionFrames(Vec<FrameInfo>)`** — present in `crates/smelt-db/src/lib.rs`; `FrameInfo` in `signatures.rs:1619` carries function name, param, rendered type, decl/call ranges.
- ✅ **Error-message format guarantees** (`expected X, got Y`, single primary span, no row variables) — covered by `function_body_check::tests` and struct/tableexpr body-check tests.
- ✅ **LSP stability under broken bodies** — probe confirmed: Tier 2 broken body → call's return degrades to `UNKNOWN` (`smelt type` over probe: `caller_t2 → {v: UNKNOWN}`); Tier 3 broken body → declared return stays stable (`caller_t3 → {v: INTEGER}`). Matches Surface §"LSP stability".

### Semantics drift

- ✅ **Tier 1 — call-site expansion** with frame trace — `CheckMode::Tier1Expansion`, covered by single/multi-frame expansion tests in `function_body_check.rs::tests`.
- ✅ **Tier 2 — isolated body check + call-site arg check** — `CheckMode::Tier2Isolated` / `Tier2CallSite`, `is_tier2_function` (`function_body_check.rs:60`).
- ✅ **Tier 3 — isolated body check against declared return** — `check_tier3_return_type` (`function_body_check.rs:2822`).
- ⚠️ **Tier 2 calling Tier 1 — inline expansion at definition time** — type-checking side covered (workspace tier-mixing tests). **Codegen/execution side is broken for nested `smelt.functions.*` calls** (see Invariant drift / BUG-013): the *run pipeline* never expands a function call that appears inside another function's body, so the transitive-chain scenario this section describes does not execute. (This is an `expansion.md`/run-pipeline codegen defect surfaced while probing the Tier 2→Tier 1 path; the type-checking semantics here are sound.)
- ✅ **No cross-boundary inference** (Tier 1 return computed per call site) — `Tier1Expansion` recomputes per call; no published synthesized signature.
- ✅ **Engine-alias normalisation** (`Text`/`Varchar`) — handled in `type_inference` / `types.md` String unification (alias table); type_property_tests green.
- ✅ **`List<Unknown>` widening** (`MetaListHeterogeneous`, `MetaListEmptyTypeUnknown`) — codes present; owned jointly with `meta_language.md`.

### Invariant drift

- ✅ **#1 Tier is a function of the signature alone** — `compute_tier` reads only `params` + `return_type_text` (no body). Upheld. (Note the *content* of that function is wrong for malformed annotations — BUG-012 — but it is still body-independent.)
- ✅ **#2 Tier 1 expansion is bounded** — cycle pre-pass (`functions.md` invariant) guarantees acyclicity.
- ✅ **#3/#4 Frame stacks populated, innermost-first** — `FrameInfo` pushed per expansion; ordering tests present.
- ✅ **#5 Tier 2/3 signatures stable under body errors** — confirmed via probe (return degrades to Unknown / stays declared; signature still drives caller checks).
- ✅ **#6 No diagnostic bypasses the format contract** — body-check tests assert `expected X, got Y`.
- ✅ **#7 Pure-function rule** — `compute_tier`, `is_tier2_function`, `check_function_body`, `check_tier3_return_type` are plain functions; no Salsa references inside.
- ❌ **Run-pipeline codegen of nested function calls** (not a numbered gradual_typing invariant, but a cross-cutting Run-Pipeline-Parity / `expansion.md` concern surfaced here) — a `smelt.define` body that calls another `smelt.define` emits the inner call **verbatim** to the engine. Root cause: `crates/smelt-dialect/src/printer.rs:205,213` reparse the expanded body via `smelt_parser::parse(&expanded)`, but the body text carries its wrapping parens (`fn_bodies.rs:69`), and `parse("(…)")` does **not** recognise `SMELT_PATH_CALL` inside a bare parenthesised fragment (only inside a `SELECT …`). So the printer's re-expansion pass is a no-op for nested calls. Confirmed for both scalar (`SELECT smelt.functions.outer(10)` → `smelt.functions.inner(10)` leaks) and FROM-position (`FROM smelt.functions.wrap_tbl(smelt.base)` → `smelt.functions.passthru(main.base)` leaks) nesting; both fail on DuckDB with `Catalog "smelt" does not exist`. → **BUG-013** (needs-review).

### Timeless-oracle drift
- ✅ No `Phase [A-Z0-9]` leakage in `docs/specs/gradual_typing.md` body. Phase numbers appear only in research cross-refs (`research §16 #16`) which are tolerated.
- ✅ No phase leakage in the referenced user-doc area.

### Freshness
- last_reviewed: 2026-05-09
- The tier-dispatch and body-check code is broadly fresh; the spec accurately describes the design. The one genuine spec-vs-code gap (BUG-012, malformed→Tier 1) predates `last_reviewed` and is a never-implemented Surface rule, not post-review drift.
- Verdict: **broadly fresh**; no `/smelt:spec` needed for staleness. The two findings are tracked in the bug ledger for the human review pass.

### Summary
- Drift items: **2** (1 Surface: BUG-012 malformed→Tier1 unimplemented; 1 cross-cutting codegen/Invariant: BUG-013 nested-fn run-pipeline expansion). Both `needs-review` (logged in `docs/bug-hunt/2026-05-30-findings.md`).
- The gradual-typing *type-checking* layer (tier dispatch, body checks, frame traces, LSP stability, format contract) is sound and well-covered. Both findings sit at boundaries: BUG-012 is the cross-crate "what counts as malformed" determination; BUG-013 is the shared run-pipeline codegen (BUG-006 class).
- Recommended next step: resolve BUG-012 / BUG-013 in the post-sweep human pass (each has code-fix vs spec/docs options in the ledger). No autonomous spec edit.
