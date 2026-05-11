---
feature: gradual_typing
status: experimental
last_reviewed: 2026-05-09
owners: [andrew]
---

# Gradual Typing

> **Scope.** Normative spec for the three annotation tiers (Tier 1/2/3), the dispatch rules that send a `smelt.define` body down the right checking path, the error-tracing contract that ties type errors back to the call site, and the LSP-stability guarantees under broken bodies. Type vocabulary, fragment sorts, and the bidirectional rule for argument/parameter checking live in `types.md` (§"Bidirectional checking"). Function declaration grammar and frontmatter live in `functions.md`. Body-scope name resolution lives in `scoping.md`.
>
> This spec governs the **process** of checking a `smelt.define`: when types push down (checking mode), when they flow up (synthesis mode), how Tier 1 errors are mapped back to call sites, and how tier mixing is handled.

## Surface

### Tier dispatch is implicit

There is no `tier:` keyword. A function's tier is derived from annotation completeness on its declared signature:

| Annotations | Tier |
|---|---|
| At least one parameter has no type annotation | **Tier 1** |
| Every parameter is annotated; no return type | **Tier 2** |
| Every parameter is annotated; return type declared (`-> <Type>`) | **Tier 3** |

`TableExpr` and `SelectItems` parameters are always considered "annotated" for the purpose of this rule (their sort is implicit from the keyword); only `Expr<T>` parameters require an explicit `: <Type>` to count. A malformed annotation (one that fails `InvalidFunctionTypeRef`) is treated as unannotated, demoting the function to Tier 1.

Tier escalation is non-breaking for callers — adding annotations to a previously Tier 1 function moves errors earlier without changing the error format (`expected X, got Y`) or the surface API.

### Diagnostic codes

This spec re-anchors the body-checking codes already catalogued in `functions.md` and adds none of its own. Each fires from one of the three tier-dispatch paths:

| Code | Tier paths that emit it |
|---|---|
| `FunctionBodyTypeMismatch` | Tier 1 expansion (with frame trace), Tier 2 isolated check, Tier 3 isolated check |
| `ReturnTypeMismatch` | Tier 3 isolated check (after body synthesises) |
| `ArgTypeMismatch` | Tier 2/3 call-site check (declared types push down) |
| `MissingArgument` | All tiers (positional/named-arg shape check) |
| `FragmentColumnMissing`, `FragmentKindMismatch` | All tiers (fragment validation runs after tier dispatch — see `scoping.md`) |

User-visible diagnostics carrying expansion-frame context are stamped with `DiagnosticData::ExpansionFrames(Vec<FrameInfo>)`. Each `FrameInfo` carries the function name, the parameter that produced the binding, the rendered concrete bound type, and optional decl/call-site ranges for the LSP `DiagnosticRelatedInformation` fan-out.

### Error-message format guarantees

Every type error is rendered against the user-visible contract from research §9:

1. **Local.** Each diagnostic has a single primary span. Multi-constraint diagnostics ("constraint A from line 5 conflicts with constraint B from line 20") are not produced.
2. **`expected X, got Y`.** Every type-mismatch message has two sides; both are concrete types (or sort signatures), never row variables.
3. **No row variables in messages.** Row variables (`Struct<{…, ..r}>`, `TableExpr<{…, ..r}>`) bind locally before any diagnostic fires (`types.md` §10). When a row binding fails, the error reports the concrete field set ("expected field `revenue`, struct has fields `{id, name}`"), not the row variable.
4. **Tier escalation never changes the format.** Moving from Tier 1 to Tier 2 to Tier 3 moves the error closer to its source and changes which span is primary; the message shape is invariant.

### LSP stability under broken bodies

A function's signature and the diagnostics it contributes to its callers depend on its tier:

- **Tier 1.** No declared signature exists independently of the body. While a Tier 1 body fails to parse or type-check, calls to it cannot be checked. Callers see `UnknownSmeltFn`-shaped degradation or a propagated body-error frame, depending on how far the parser got.
- **Tier 2.** The declared parameter types form a stable signature. Calls continue to type-check against the declared parameters even when the body is broken mid-edit. The synthesised return type at the call site degrades to `Unknown` (so downstream chains keep flowing), but `ArgTypeMismatch` at the call site fires normally.
- **Tier 3.** Same as Tier 2 plus a stable declared return type. Hover, downstream type inference, and `ReturnTypeMismatch` at the body all continue to work even when the body fails to synthesise.

This is a load-bearing property of the gradual-typing thesis: shared/library functions live at Tier 2/3 precisely so their callers do not break under in-progress edits.

## Semantics

These rules are normative.

### Tier 1 — call-site expansion

A Tier 1 `smelt.define` has no usable signature in isolation. Checking happens **at each call site**:

1. The compiler collects the concrete argument types at the call (synthesised from the caller's `TypeContext`).
2. It binds the callee's parameter names to those argument types and walks the callee's body in **synthesis mode** under that binding (the `Tier1Expansion` `CheckMode` of `function_body_check.rs`).
3. Any diagnostic produced by the body walk is re-anchored: its primary span stays at the offending sub-expression, and a `FrameInfo` is pushed onto its `DiagnosticData::ExpansionFrames` carrying the callee name, the parameter whose binding produced the type that caused the error, the rendered concrete type, and the declaration / call-site ranges.
4. A synthesised return type flows up from the body and is consumed by the surrounding expression context.

The body is checked **once per call site**. A Tier 1 callee called from N distinct call sites runs N independent expansion checks; their diagnostics are not deduplicated. (Salsa caching coalesces work across re-runs of the same call site, not across different ones.)

### Tier 2 — isolated body check + call-site argument check

A Tier 2 `smelt.define` is checked **in isolation** at definition time:

1. The body is walked in **synthesis mode** under a `TypeContext` whose `function_params` map is seeded from the declared parameter signatures (`Tier2Isolated` `CheckMode`). The synthesised return type is not validated against any declared return — there is none.
2. Any error in the body fires at definition time as `FunctionBodyTypeMismatch`, with no frame trace (the body is its own scope).
3. At each call site, the declared parameter types are pushed into the corresponding arguments in **checking mode** (`Tier2CallSite` `CheckMode`); a mismatch is `ArgTypeMismatch`. No expansion happens at the call site.

Row-polymorphic parameters (`TableExpr<{…, ..r}>`, `Expr<Struct<{…, ..r}>>`) are unified **locally per call site** against the concrete argument schema; the row variable binds at that call only and propagates into the call's synthesised return type. The body check (step 1) sees the row variable as abstract — it must verify the body uses only the declared fields, never the row tail.

### Tier 3 — isolated body check against declared return + call-site argument check

A Tier 3 `smelt.define` extends Tier 2 with a declared return type:

1. The body is walked in **checking mode** against the declared return type. The body's synthesised type must match the declared return; a mismatch fires `ReturnTypeMismatch` at the body.
2. Argument-side call-site checking is identical to Tier 2.
3. Hover and downstream inference resolve the call's return type from the declared signature, never by expansion.

### Tier 2 calling Tier 1 (research §16 #17)

When a Tier 2 (or Tier 3) body calls a Tier 1 helper, the Tier 1 callee's body is **expanded inline during the Tier 2 body check**. The mechanism:

1. The Tier 2 body check, walking under its declared parameter types, reaches a call to a Tier 1 function `f`.
2. The checker collects argument types from the surrounding Tier 2 `TypeContext`.
3. The Tier 1 expansion routine (Tier 1 path above) runs for `f` with those types as concrete bindings.
4. The synthesised Tier 1 return type flows back into the Tier 2 body check at the call position.
5. Any diagnostic from the Tier 1 expansion is reported against the Tier 2 body, with `ExpansionFrames` rooted at the Tier 2 call site to `f`.

Transitive Tier 1 → Tier 1 chains compose: every level expands under the types derived from the Tier 2 root. Termination is guaranteed by the §3 cycle rule from `functions.md`.

Tier 2 → Tier 2 and Tier 2 → Tier 3 calls remain **signature-only**: no expansion, the declared signature is consulted, and the call is checked exactly like a built-in. This preserves Tier 2/3's LSP-stability guarantee for callees that are themselves stable.

### No cross-boundary inference

A Tier 1 function's return type is **computed at each call site** from the expansion. It is never inferred-then-propagated as if it were a declared signature. This is the rule that keeps errors local: the alternative — letting Tier 1 callees publish a synthesised return — would create non-local errors when an unrelated change to the callee's body changes the call site's inferred type.

A Tier 1 function that wants a stable, declared return type must escalate to Tier 3.

### Engine-alias normalisation

Type comparison treats engine aliases as equal: `Text` and `Varchar` unify; `Integer` and the engine's `INT` unify. Normalisation happens before the diagnostic message is rendered, so users never see a `Text vs Varchar` mismatch in error text. (See `types.md` §"String unification" for the full alias table.)

### `List<Unknown>` widening (meta-language)

`List<T>` is meta-only (defined in `meta_language.md`). It interacts with this spec's `Unknown` widening discipline at three points:

1. **Heterogeneous list literal degrades to `List<Unknown>`.** A list literal whose elements do not unify under the LUB rules of `types.md` §"Numeric promotion chain" emits `MetaListHeterogeneous` (anchored at the literal's source span) and produces a `List<Unknown>` value. The body that consumed the literal continues to type-check; downstream operations against `List<Unknown>` follow the standard `Unknown` rules below.
2. **Empty literal in unknown-target position degrades to `List<Unknown>`.** `[]` at a position with no inferable target sort emits `MetaListEmptyTypeUnknown` and produces `List<Unknown>`. Same downstream consequences.
3. **`List<Unknown>` propagation.** Operations over `List<Unknown>` produce `Unknown` (HOF results), `Unknown`-elemented results (further `List<Unknown>`), or are a `TypeMismatch` when the operation requires a satisfied constraint (`reduce` with a numeric reducer over `List<Unknown>` is a `TypeMismatch`, *not* a silent fallback to a different reducer). Spread of a `List<Unknown>` into a comma-separated position emits each element as an `Unknown`-typed splice; existing kind-ceiling checks at the splice position then run normally and may produce further diagnostics. No new diagnostic codes are introduced for `List<Unknown>` propagation; the existing `TypeMismatch` / `Unknown` rules in `types.md` apply.

The rule that **`List<Unknown>` does not silently become `List<Any>`** is load-bearing: `Any` accepts every type and would mask the upstream LUB failure forever; `Unknown` is the compiler's "we already told you about this" type and continues to surface upstream errors at every consumer. The widening diagnostic codes (`MetaListEmptyTypeUnknown`, `MetaListHeterogeneous`) fire exactly once at the source of the widening, and downstream surface only the consumer-side `TypeMismatch` if any.

### What gradual typing does **not** do

These exclusions are normative — they bound what future plans may add without revisiting this spec:

- **No higher-rank polymorphism.** A user `smelt.define` parameter is not itself polymorphic. (`<T: Constraint>` generics live in built-ins and `smelt.extern` only — see `types.md` §"Generics inference".)
- **No global constraint solving.** Unification is local per call site. There is no Hindley-Milner Algorithm W, no occurs check, no let-generalisation.
- **No implicit cross-type coercion.** When the caller passes `Expr<Integer>` to an `Expr<Double>` parameter, this is a type error; the caller writes `CAST(x AS DOUBLE)`. (Built-in operators that compute the LUB of multiple numeric arguments are a property of those built-in signatures — `types.md` §"Numeric promotion chain" — not implicit coercion at the user-function boundary.)
- **No nullability in user signatures.** `Expr<T>` parameters and returns are implicitly nullable in v1. The column-level `nullable` flag flows through inference but does not surface in `smelt.define` annotations.
- **No `Any` / unknown escape hatch.** A function the checker cannot type degrades by surfacing the underlying error or by losing precision on its return type to `Unknown`; there is no opt-out annotation that suppresses checking.

### Interactions with adjacent specs

- **`types.md` §"Bidirectional checking"** owns the rule that types push down at calls and flow up in bodies. This spec governs the *dispatch* of which body walks under which mode for which tier; it does not restate the bidirectional rule.
- **`scoping.md`** owns parameters-first lookup, the no-overlap rule, and splice-context inference. The tier-dispatch path determines what `TypeContext` seeds the body walk; what *names* resolve in that walk is `scoping.md`'s territory.
- **`functions.md`** owns the declaration grammar and the `FunctionCallCycle` cycle pre-pass — gradual typing assumes acyclic call graphs and does not re-justify them here.
- **`expansion.md`** owns the AST-level expansion mechanics, hygiene, and provenance origin tags. This spec consumes the frame-stack data structure but does not specify it.

## Design

This section captures the load-bearing rationale behind the tier model and the bidirectional-checking discipline above. Where deeper justification exists, it lives in `docs/research/20260413-smelt-functions.md` §8–§9 and §16, and is cross-linked.

**Three tiers, not "annotate everything" or "infer everything".** The SQL ecosystem has two real audiences: pole users iterating in a notebook (Python-style, no annotations, fast feedback) and contract authors of shared/library functions (Haskell-style, full annotations, stable signatures under in-progress edits by other people). A binary choice loses one camp — pure inference fails the contract author because library callers see signatures change under them, and mandatory annotation fails the pole user because every throwaway helper costs ceremony. Three tiers let users pay annotation cost only where it earns LSP stability or call-site type errors. Tier 1 is for "I want this thing to work right now"; Tier 2 is for "callers should get arg-type errors locally and not break under my body edits"; Tier 3 adds "downstream inference and hover survive a broken body". The escalation path is monotonic and non-breaking — adding annotations strictly improves diagnostics without changing the surface (research §8).

**Tier dispatch is a function of the signature, never the body.** The body is mutable; the signature is the contract. If body content could change tier classification, an unrelated body edit would silently change the function's checking discipline (and therefore its callers' diagnostics) without any spec-level signal. Computing tier from declared annotations only — `compute_tier(FunctionSig)` — keeps the dispatch stable across body iteration and makes "what tier is this?" answerable from the signature alone in the LSP. The alternative (body-aware dispatch — e.g. "if the body has a return type that's inferable, treat as Tier 3") was rejected because it conflates contract and implementation: the tier is about what callers see, and that has to be determined before the body is even parsed (see Constraint 1).

**Tier 1 expands per call site instead of generalising a signature.** A Hindley–Milner-style approach would unify across call sites and synthesise one row-polymorphic signature for a Tier 1 function. That signature would then have to surface in user-facing diagnostics — "expected `{a: Int, b: Text, ..r}`, got `{a: Int}`" is hard to read for SQL users and harder to fix. Per-call-site expansion side-steps the problem: each call sees concrete types in, concrete types out, and any error inside the expanded body reports against the concrete bound types rather than abstract row variables. The cost is that Tier 1 functions called from N call sites pay N independent body checks; Salsa caching softens the constant factor, and authors who care about that cost have a clean escape hatch — escalate to Tier 2 (research §9, §16 #16).

**Bidirectional, not pure synthesis.** Declared types push down at call sites and into bodies (checking mode), and bodies synthesise types upward where no expectation exists (synthesis mode). This makes most error messages local: a Tier 2 call-site argument mismatch fires at the argument expression with the declared parameter type as `expected`; a Tier 3 body return mismatch fires at the body's tail with the declared return type as `expected`. Pure synthesis with global constraint solving (Algorithm W) would surface unification errors far from their source — a row variable bound at line 5 and contradicted at line 80 produces a "constraint A from line 5 conflicts with constraint B from line 80" message with no obvious primary span. Bidirectional checking gives every diagnostic exactly one primary span, which is the format contract callers actually see, and lets row variables stay internal to local unification (research §9, see also `types.md` §"Bidirectional checking").

**Tier 2 calling Tier 1 inlines at definition time, not opaquely.** When a Tier 2 body calls a Tier 1 helper, the Tier 1 callee is expanded inline under the Tier 2 body's type context — and any error from inside the Tier 1 expansion is anchored back at the Tier 2 call site. Treating Tier 1 as opaque-from-Tier-2 (so the Tier 2 body sees only `Unknown` from the Tier 1 call) was rejected because it makes Tier 2 errors confusingly nonlocal: a Tier 2 author sees `ArgTypeMismatch` at the call to `f` but the actual cause is buried inside `f`'s body, with no way to discover the binding chain without a frame trace. Inline expansion preserves the locality contract — the user sees the Tier 2 boundary they wrote with frame breadcrumbs into the Tier 1 helper. Tier 2 → Tier 2 and Tier 2 → Tier 3 calls remain signature-only because those callees have a stable contract by definition; inlining them would dilute the LSP-stability guarantee (research §16 #17).

**No cross-boundary inference of Tier 1 return types.** A Tier 1 function's return type is "whatever the body produces under these argument types at this call". Propagating that synthesised return as if it were a declared signature would create implicit cross-call dependencies: an unrelated body edit changes the inferred return at one call site, which changes inference downstream at *that* call site, which produces a new error elsewhere — and none of it is visible at the spec/signature level. Recomputing per call keeps every Tier 1 inference local and reproducible from the spec alone. The escape hatch for users who want a stable, declared return is explicit annotation — escalate to Tier 3 (research §8, §9).

**Row variables must never appear in user-facing messages.** Row polymorphism is an internal mechanism for typing helpers like `add_margin(source: TableExpr) -> TableExpr AS (SELECT source.*, revenue - cost AS margin FROM source)` — the row tail is bookkeeping, not user surface. If users had to read row variables in error messages, they would have to learn what they mean, which contradicts gradual typing's "pay only what you earn" thesis. Even letting row variables leak in expert-mode diagnostics was rejected: SQL users — including expert ones — do not think in row-polymorphic terms, and asking them to is a mismatch with the language's mental model. Row variables therefore bind locally before any diagnostic fires, and unification failures report against the concrete field set ("struct has fields `{id, name}`, missing `revenue`") (research §9, see also `types.md` §"Row polymorphism").

**Frame stack is populated to full depth even though v1 renders only one level.** Multi-level rendering — "in expansion of A → B → C, the binding `x: Int` at A → B caused …" — is the correct UX for nested Tier 1 chains, but the renderer is diagnostic polish, not a soundness gap. The frame data structure is cheap to produce during expansion (one push per call) and expensive to retrofit after the fact (every expansion site would otherwise need re-instrumenting). Populating it eagerly at full depth means future renderer phases read more of the same data structure without changes elsewhere — a one-pass renderer upgrade rather than a re-instrumentation pass. v1 renders outermost-call + innermost-error; the rest is data-on-disk waiting for the renderer to catch up (research §16 #16).

**Engine aliases (`Text`/`Varchar`) treat as equal at unification.** A function body that compiles against DuckDB should not fail when the workspace switches to PostgreSQL because of a cosmetic type-name mismatch (`Text` vs `Varchar`, `Integer` vs `INT`). Strict equality with explicit casts was considered and rejected because every cross-engine helper would need boilerplate `CAST(x AS TEXT)` calls that say nothing semantically — the canonical type vocabulary already handles this, and unification that respects the alias table lets the canonical types do their job. The full alias table lives in `types.md` §"String unification" and is shared across the type checker; this spec only commits to the rule that aliases are equal *before* the diagnostic message is rendered, so users never see `Text vs Varchar` in error text.

## Constraints & Invariants

1. **Tier is a function of the signature alone.** Two `smelt.define`s with identical signatures produce identical tier dispatch regardless of body content. The body never re-enters tier classification.
2. **Tier 1 expansion is bounded.** The transparent-function call graph is acyclic (`functions.md` invariant 2); every Tier 1 expansion terminates.
3. **Frame stacks are populated for every call expansion.** `FrameInfo` is pushed on every Tier 1 (or Tier 2 → Tier 1) expansion frame, regardless of whether the renderer in v1 reads it. Future renderer upgrades (multi-level rendering — see Known Divergences) read more of the same data structure without changes elsewhere.
4. **Frame ordering is innermost-first → outermost-last.** `frames.first()` is the deepest nested call; `frames.last()` is the outermost call site (the source span the user wrote). Renderers walk the vector in reverse for outer-to-inner presentation.
5. **Tier 2/3 signatures are stable under body errors.** A `FunctionBodyTypeMismatch` or `ReturnTypeMismatch` at a Tier 2/3 body must not invalidate the signature for caller-side checking. This is the LSP-stability invariant.
6. **No diagnostic bypasses the format contract.** Every type-error diagnostic is rendered as `expected X, got Y` (or "expected field X, struct has fields Y") with concrete types — including diagnostics fired from inside an expanded Tier 1 body.
7. **Pure-function rule.** Tier dispatch (`compute_tier`, `is_tier2_function`), body checking (`check_function_body`, `check_smelt_fn_call`), and tier-3 return-type checking (`check_tier3_return_type`) are pure functions of `FunctionSig`, `Expr` / `SelectStmt`, `TypeContext`, and source text — no Salsa references inside the analysis logic. (Architecture invariant from `architecture.md` and CLAUDE.md.)
8. Out of scope for v1 (intent — preserved here so future plans honour it):
   - Multi-level frame rendering (planned, currently single-level — see Known Divergences).
   - Cross-call inference of Tier 1 return types.
   - Tier-ordering invariants ("Tier N may only call ≥ Tier N").
   - An `Any`-typed escape hatch.
   - Caching of Tier 1 expansion checks across multiple Tier 2 body re-checks (orthogonal to correctness; covered as deferred polish).

## Known Divergences / Open Questions

- **Multi-level frame rendering is deferred** (research §16 #16). The frame stack is populated to arbitrary depth (constraint 3 above), but the LSP renderer reads only the outermost call site and the innermost error in v1. For nested chains (A → B → C), the user sees A's call site and C's error with the binding at the innermost frame; the intermediate "in expansion of B" line lands in a follow-up renderer phase. This is diagnostic polish, not a soundness gap.
- **LSP hover for Tier 1 return types** depends on a successful expansion at the hover position. If the surrounding context cannot be type-inferred (e.g. inside a broken sibling expression), Tier 1 hover may show `Unknown`. Tier 2/3 hover is unaffected. The exact live behaviour should be audited against this spec.
- **Tier 1 → Tier 2 upgrade-path breaking changes** are an open item from research §16 #17. Annotating a previously Tier 1 parameter with a type narrower than what some caller was passing will surface a new `ArgTypeMismatch` at that caller. There is currently no migration tooling for this; it is a known sharp edge of the upgrade story.
- **Caching of Tier 1 expansion checks** across re-runs of the same Tier 2 body is not currently exploited beyond Salsa's per-input memoisation. Whether to add a dedicated expansion cache (keyed on (callee_id, arg_types)) is open and orthogonal to correctness.
- **Diagnostic deduplication across distinct call sites.** A Tier 1 callee with a body bug today emits one `FunctionBodyTypeMismatch` per call site that reaches it. Whether to dedupe these per `(file, body-span)` is a polish item flagged under research §16 #16.
- **Diagnostic codes pre-`diagnostics.md`.** Codes listed in this spec are owned here until a `diagnostics.md` spec lands. `diagnostics.md` will define ownership rules, severity tiers, stability tiers, and suppression. Code names may be renamed under that spec. (See `architecture.md` §"Specs not yet authored".)

## References

### Code

- `crates/smelt-types/src/signatures.rs` — `Tier`, `FunctionSig::tier`, `compute_tier`, `FrameInfo`
- `crates/smelt-db/src/function_body_check.rs` — `CheckMode` (`Tier1Expansion` / `Tier2Isolated` / `Tier2CallSite`), `is_tier2_function`, `check_function_body`, `check_function_body_with_expansion`, `walk_body_with_ctx`, `check_smelt_fn_call`, `check_tier3_return_type`, `NestedCallHandler`
- `crates/smelt-db/src/lib.rs::DiagnosticData::ExpansionFrames` — frame-stack payload on diagnostics
- `crates/smelt-db/src/type_inference.rs` — synthesis-mode walk consumed by tier dispatch (`infer_expression_type`, `TypeContext::function_params`, `TypeContext::expected_return`)

### Tests

- `crates/smelt-db/src/function_body_check.rs::tests` — tier classification, Tier 2 isolated check, Tier 3 return-type validation, single-frame and multi-frame expansion traces
- `crates/smelt-db/tests/` — workspace-level tier-mixing tests (Tier 2 → Tier 1 inline expansion, frame-stack stamping)
- `examples/test_workspace/functions/` — worked tier examples consumed by the LSP-diagnostics integration test

### User docs

- `docs-site/docs/concepts/functions.md` and adjacent typing pages — to be reconciled against this spec via `/smelt:validate gradual_typing`

### Plans (history) — oldest → newest

- `docs/plans/20260422-smelt-functions.md` — primary implementation plan; Phases 5, 6, 12, 25, 26 cover the surface in this spec
- `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file

### Related specs

- `docs/specs/types.md` — type vocabulary, fragment sorts, bidirectional checking rule, generics inference
- `docs/specs/functions.md` — declaration grammar, frontmatter, function-level diagnostics, cycle rule
- `docs/specs/scoping.md` — body-scope name resolution and the `TypeContext` seeding contract
- `docs/specs/expansion.md` — AST-level expansion mechanics, provenance origin tags, hygiene
- `docs/specs/meta_language.md` — `List<Unknown>` widening interacts with the meta-language list surface; this spec owns the widening discipline, `meta_language.md` owns the diagnostic codes

### Research

- `docs/research/20260413-smelt-functions.md` — sections 8 (Three Tiers), 9 (Bidirectional Checking), 16 decisions 16 (Tier 1 single-level traces) and 17 (Tier 2 calling Tier 1) are the source for this spec
