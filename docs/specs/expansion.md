---
feature: expansion
status: experimental
last_reviewed: 2026-05-13
owners: [andrew]
---

# Expansion

> **Scope.** Normative spec for the AST-level expansion mechanics that back `smelt.define` calls: the two senses of "expansion" (type-check-time binding versus codegen-time CST rewrite), the provenance origin tags attached to expanded nodes, the frame-stack data structure consumed by diagnostics, and v1 hygiene (parameters-first lookup at type-check time, CTE-collision diagnostic instead of alpha-rename).
>
> Most of this spec describes an **implementation invariant**, not a user-visible surface. The user-visible bits are exactly two: the rendered expansion-frame trace on type errors (already specified in `gradual_typing.md` §"LSP stability under broken bodies" and §"Tier 1 — call-site expansion") and the `CteCycle` / future CTE-collision diagnostic. This spec is short on purpose. It exists so future planner work cannot silently drop the provenance contract.

## Surface

### User-visible artefacts

The following are the only outputs of expansion that a user can observe:

- **Expansion-frame messages on diagnostics.** When a Tier 1 expansion causes an error to fire inside an expanded body, the diagnostic carries `DiagnosticData::ExpansionFrames(Vec<FrameInfo>)`. The LSP renderer turns this into a trailing `in expansion of <fn>, <param> was bound to <type>` line and (for clients that consume it) a `DiagnosticRelatedInformation` link per frame pointing at the declaring file. The format guarantees and the per-tier conditions under which frames appear are normative in `gradual_typing.md`; this spec only specifies the underlying data structure.
- **CTE-collision diagnostic.** When a CTE name introduced inside a function body would collide with a CTE in the calling scope at codegen time, the compiler emits a diagnostic rather than alpha-renaming. The exact code surfaces under `scoping.md`'s `CteCycle` family in v1 (or a dedicated code minted in a follow-up); the v2 design replaces this diagnostic with an automatic alpha-rename — see Known Divergences.

### `FrameInfo` shape

`FrameInfo` is the public shape of a single frame. The struct lives in `smelt-types` because both `smelt-db` (which produces frames) and `smelt-lsp` (which renders them) consume it without a Salsa dependency:

| Field | Meaning |
|---|---|
| `function` | Name of the function whose expansion produced the frame. For HOF anonymous frames this is the bracketed HOF name (`"<map>"`, `"<filter>"`, `"<reduce>"`). The angle brackets are part of the value and distinguish synthesised HOF frames from user-named function frames. |
| `param` | Name of the parameter whose binding produced the inner error. Empty string for anonymous HOF frames. |
| `bound_type` | Concrete bound type, rendered for display via `DataType::to_string()`. Empty string for anonymous HOF frames. |
| `decl_path` | Path to the file declaring the function (`Option`; `None` on degraded sig-lookup and always `None` for anonymous HOF frames). |
| `decl_range` | Source range of the declaration's name token. `None` for anonymous HOF frames. |
| `call_site_range` | Source range of the call-path span that produced this frame. For HOF frames this is the span of the HOF call expression. |
| `fn_id` | Function identifier in the registry (`Option<String>`). `None` for anonymous HOF frames — HOFs are built-in and have no declaring file. `Some(name)` for `smelt.define` frames. |
| `element_index` | Zero-based index into the source list literal, identifying which element the lambda body operated on. `None` when the source was not a literal or the information is not statically available (the common v1 case). |

**Anonymous-frame form** (HOF inline-expansion): `map`, `filter`, and `reduce` produce anonymous frames when a type error surfaces inside a lambda body. The anonymous form has `fn_id = None`, `decl_path = None`, `decl_range = None`, and `param = ""`. The `function` field carries the angle-bracketed name (`"<map>"`, `"<filter>"`, `"<reduce>"`). Renderers should handle `fn_id = None` by omitting the "defined in" link and displaying only the bracketed HOF name (e.g. `in expansion of <map>`).

**`column_origin` on anonymous HOF frames.** When the HOF source list comes from `smelt.columns_of(t)`, the anonymous frame is extended with an optional `column_origin: Option<TextRange>` field carrying the source span of the column's declaration in the upstream `ModelSchema`. Producers populate `column_origin` per-element when the column's source span is statically resolvable; `column_origin = None` when the source was not a `smelt.columns_of` call, the schema was unresolvable, or the span is unavailable. The v1 LSP renderer does not yet surface `column_origin` (tracked in Known Divergences); it is a producer-side invariant only.

**`model_origin` on anonymous HOF frames.** When the HOF source list comes from `smelt.models.with_tag(…)` or `smelt.models.all`, the anonymous frame is extended with an optional `model_origin: Option<ModelOrigin>` field. `ModelOrigin` carries `path: String` (the workspace-relative path to the model's `.sql` file with `/` separators) and `frontmatter_span: Option<TextRange>` (the source span of the frontmatter block, or `None` when the model has no frontmatter or the span is unavailable). Producers populate `model_origin` per-element; `model_origin = None` when the source was not a `smelt.models.*` call or the model's path is not resolvable. The v1 LSP renderer does not yet surface `model_origin` (tracked in Known Divergences); it is a producer-side invariant only.

**`source_origin` on anonymous HOF frames.** When the HOF source list comes from `smelt.sources.with_tag(…)` or `smelt.sources.all`, the anonymous frame is extended with an optional `source_origin: Option<SourceOrigin>` field. `SourceOrigin` carries `path: String` (the workspace-relative path to the sources YAML file) and `declaration_span: Option<TextRange>` (the source span of the source's entry in the YAML, or `None` when the span is unavailable). Producers populate `source_origin` per-element; `source_origin = None` when the source list was not a `smelt.sources.*` call. The v1 LSP renderer does not yet surface `source_origin` (tracked in Known Divergences); it is a producer-side invariant only.

**`<generator>` frame for multi-model production.** A generator file (a file whose frontmatter declares `generates: models`, per `meta_language.md` §"Multi-model production") produces an outermost frame stamped with `function = "<generator>"`, `fn_id = None`, `decl_path = Some(<generator file path>)`, `decl_range = None`, `call_site_range = <range of the file's body expression>`, and `param = ""`. Diagnostics that surface from inside the generator body's HOF chain (e.g. `RecordFieldMissing` on a `ModelDef` literal, `MapGetMissingKey` on the loaded config, `ColumnsOfRequiresTableExpr` deep in a lambda) carry this `<generator>` frame at `frames.last()` (outermost). Inner HOF anonymous frames remain in place at `frames[0]` (innermost). Structural file checks (e.g. `ModelDefDuplicateName`, `GeneratorBodyForbidsModelReflection`) do not arise from an expansion and therefore carry no frame stack, per invariant 4. The `<generator>` frame is the wide-reflection-style provenance root for emitted-model diagnostics; its `decl_path` resolves to the generator file the user can navigate to. The v1 LSP renderer reads `decl_path` and `call_site_range` for the `<generator>` frame. The `model_origin` extension (a `Some(ModelOrigin)` carrying the offending `ModelDef.name` range) is **not yet populated by the producer** in v1 — populating it requires per-diagnostic stamping which the current single-frame-per-file constructor does not supply; tracked in Known Divergences.

The renderer is permitted to read fewer fields than are populated; producers must populate every field they have available regardless of what the current renderer reads.

## Semantics

These rules are normative.

### Two senses of expansion

A `smelt.define` call participates in two expansion paths during a single compile, and they have different mechanisms:

1. **Type-check-time expansion (Tier 1 only).** Performed during diagnostic-collection over a function body that calls another function. The compiler binds the callee's parameter names to the caller-supplied argument types in a `TypeContext` and re-walks the callee body under that context. **No CST is rewritten.** The body is read in place; argument types flow in via the type context.
2. **Codegen-time expansion (all tiers).** Performed at the planner's Level-2 strategy lowering when a transparent function call is materialised into SQL. The compiler clones the callee body's CST, substitutes the caller's argument subtrees into parameter placeholder nodes, and attaches provenance tags to every node in the resulting tree. The output is a fresh CST suitable for SQL emission.

Tier 2 and Tier 3 calls are **never** expanded for type checking — the declared signature is sufficient (`gradual_typing.md` §"Tier 2" / §"Tier 3"). They are still expanded at codegen time for transparent execution.

### Provenance origin tags

Every node in a codegen-expanded CST carries an **origin tag** identifying where the node came from:

- `Caller(span)` — the node came from the caller's source (the argument subtree, or the surrounding call-site context spliced around it).
- `Callee(fn_id, span)` — the node came from the callee body's source. `fn_id` identifies the declaring function; `span` is the original location in the callee file.
- `Synthesized(fn_id, reason)` — the node was generated by the compiler with no direct source counterpart. Sources of synthesis: row-variable erasure (`gradual_typing.md` / `types.md`), default-value insertion, and planner-side strategy injection. `reason` is a structured tag describing which mechanism produced the node.

Origin tags are attached at expansion time and must propagate through every subsequent CST transformation (planner rewrites included). A planner rule that produces a new CST node must label it `Synthesized` (or copy an existing tag forward) — it must not silently emit untagged nodes.

### Frame-stack invariants

Whenever a diagnostic surfaces from inside an expanded body, the compiler stamps `DiagnosticData::ExpansionFrames(Vec<FrameInfo>)` onto the diagnostic. The vector obeys these rules:

1. **One frame per expansion level.** Every Tier 1 expansion frame pushes exactly one `FrameInfo`. A diagnostic that surfaces inside a chain `A → B → C` carries three frames.
2. **Innermost-first → outermost-last ordering.** `frames.first()` is the deepest nested call (the one closest to the inner error); `frames.last()` is the outermost call site (the source span the user wrote). Renderers presenting outer-to-inner walk the vector in reverse.
3. **The frame stack is populated even when the renderer ignores it.** A v1 LSP renderer reading only `frames.last()` is permitted; a producer that populates only the outermost frame is **not** permitted. Multi-level rendering reads more of the same vector without changes elsewhere.
4. **A diagnostic that did not arise from an expansion carries no `ExpansionFrames` payload.** The `data` field is left `None`. A defensively-pushed empty frame stack is forbidden — it would mislead the renderer into showing an empty "in expansion of" line.
5. **Frame ordering is deterministic.** When a single frame's `param_bindings` contains multiple bindings (e.g. several parameters bound at the same call), the producer chooses an order — currently the parameter-declaration order via `frame_bindings.first()` — and the renderer relies on that determinism for snapshot stability.
6. **HOF anonymous frames obey the same stack-ordering rules.** A lambda body type error inside `map(xs, fn c => bad_expr)` carries one anonymous frame at `frames[0]` (innermost). If the `map` call itself is inside a `smelt.define` body that was expanded at Tier 1, the HOF anonymous frame is prepended (innermost) and the enclosing `smelt.define` frame follows it. The renderer walks the same vector in reverse to present outermost-first.

### Hygiene v1

v1 hygiene is intentionally minimal:

1. **Parameters-first resolution at type-check time.** The parameters-first rule (`scoping.md` §"Resolution order") is enforced through `TypeContext::lookup_identifier` — no token-level rewriting, no alpha-rename. Because parameter names are resolved against the function's declared parameter map before any FROM-scope lookup, a parameter cannot be shadowed by a same-named column from a `TableExpr` parameter or a CTE.
2. **Parameter placeholders are distinct CST node kinds.** At codegen-time CST rewrite, parameter references are recognised by their CST node kind, not by their token text. They cannot collide with caller identifiers regardless of what the caller named its variables.
3. **CTE-name collisions emit a diagnostic.** When a function body declares a CTE whose name would collide with a CTE in the caller's scope at codegen time, the compiler emits a collision diagnostic anchored at the body CTE's declaration. v1 makes no attempt to rename. Authors avoid the diagnostic by choosing distinct CTE names.

These three rules together preserve correctness for v1: nothing inside an expanded body refers to a name resolved against the wrong scope. They do not deliver full referential transparency under expansion — that arrives with alpha-rename in v2 (see Known Divergences).

### Why this matters as a spec

The provenance frame stack is the bridge between Tier 1's "expand and check" semantics (`gradual_typing.md`) and the user-visible "in expansion of …" trace. A planner pass that fuses two transparent functions, or a future strategy that materialises a function call into a different shape, **must** preserve the frame contract:

- Diagnostics arising from rewritten code must carry frames whose call-site ranges still resolve to source the user wrote.
- Synthesised nodes introduced by the planner must carry `Synthesized(fn_id, reason)` origin tags so that source-mapping back to user code is possible.
- A planner pass that drops origin tags or fails to push a frame at an expansion boundary breaks the diagnostic contract silently — there is no runtime check that catches it. This is why the contract is normative here rather than left implicit in code.

## Design

This section captures the load-bearing mechanical decisions. Most of expansion is internal invariants; the rationale below is short on purpose. Deeper justification lives in `docs/research/20260413-smelt-functions.md` §16 #12.

**Two senses of expansion, two mechanisms.** Type-check-time expansion (Tier 1) needs only the parameter→type binding to walk the callee body under the caller's argument types — no CST mutation is required. Codegen-time expansion needs the rewritten CST so the dialect printer can emit a single SQL statement. Collapsing both to "always rewrite at type-check" was rejected because the architecture spec explicitly disallows CST mutation in the type-check pipeline (`architecture.md` §"CSTs are not mutated"). The opposite collapse (always defer expansion until codegen) was also rejected: Tier 1 must walk the expanded body to type-check at all (`gradual_typing.md` §"Tier 1 — call-site expansion"). Two senses, two mechanisms, sharing the same provenance vocabulary.

**Provenance is per-node, not per-frame.** Planner passes can move, drop, or split nodes from an expanded body — fusion across function boundaries, predicate pushdown across a transparent call, dead-branch elimination. Per-node origin tags survive all three operations; per-frame attribution would lose precision the moment the first rewrite ran (a node moved out of its frame keeps its tag, but a frame that owns "all nodes from this expansion" loses meaning when the planner splits the expansion). The trade-off is an authoring invariant for planner-rule writers — every CST clone has to propagate tags — captured in `planner_integration.md` as part of the provenance contract. Constraint 2 below makes the silent-drop case a spec-level bug.

**Frame stack is innermost-first → outermost-last.** Matches how a reader scans a stack trace — the deepest frame is "where it broke", outer frames are "how we got here" — and matches Rust/Python conventions. The renderer reverses for outer-to-inner display when the chosen format wants it (`gradual_typing.md` Tier 1 frame-trace rendering). Outermost-first ordering was rejected because it forces the renderer to walk to the bottom of the vector to find the actual point of failure, and because snapshot stability is easier when the producer's order matches the natural diagnostic-construction order ("push on enter, the deepest is on top of the stack").

**`Synthesized` exists alongside `Caller` / `Callee`.** Some codegen-time nodes have no source span at all — implicit CTE wrappers, CAST insertions for canonical return types, default-value expansions, row-variable erasure. Lumping them into `Callee` would lie about where to highlight in the editor (the user clicks through to a span that has nothing to do with the synthesised node); lumping into `Caller` would mis-blame the user for compiler-introduced code. The `reason` field on `Synthesized` gives the renderer enough context to choose a sensible message and gives planner authors a place to record provenance for nodes they introduce. The taxonomy of `reason` values is deliberately not enumerated in v1 (see Known Divergences).

**Hygiene is at type-check time, not at parse time.** The parameters-first rule from `scoping.md` is enforced through `TypeContext::lookup_identifier` during `function_body_check`, not by macro-style token rewriting in the parser. This keeps `smelt-parser` simple — it does not need callee-parameter-scope knowledge to parse a function body, which is essential for the LSP's error-recovery contract. Parse-time substitution was rejected because the parser does not have call-site context (which callee, which arguments) when it walks a body, and gathering that context would require a multi-pass parser. The CST stays the user's source; meaning is layered on top.

**CTE alpha-rename is deferred to v2.** v1 ships the collision diagnostic and stops there. Alpha-rename is correct in principle but interacts with the planner's provenance contract (renamed CTE nodes must carry origin tags that survive the rename and still resolve back to the user's source) and with `goto-definition` (which CTE the user clicks on when the rendered name is synthetic). Shipping the diagnostic first lets the v1 provenance contract solidify before the rewrite hits it; the deferral is recorded in Known Divergences.

## Constraints & Invariants

1. **Expansion is AST-level, never textual.** Codegen-time expansion clones CST nodes. Type-check-time expansion never rewrites the CST at all. Textual substitution of arguments into a string body is forbidden.
2. **Origin tags are conservative.** Every CST node produced by expansion carries exactly one origin tag. A node without a tag is a bug.
3. **The frame stack is innermost-first.** This direction is part of the public contract — `smelt-lsp` reverses the vector for outer-to-inner display, and snapshot tests pin the order. Reversing the producer's order breaks the renderer.
4. **`FrameInfo` is Salsa-free.** It lives in `smelt-types` and is used by both `smelt-db` and `smelt-lsp` without either crate gaining a Salsa dependency on the other. (Architecture invariant from `architecture.md` and the pure-function rule from `CLAUDE.md`.)
5. **Type-check-time expansion is bounded by the cycle pre-pass.** The `FunctionCallCycle` rule (`functions.md`) guarantees the call graph is acyclic; expansion therefore terminates without a depth limit.
6. **Codegen-time expansion is lazy.** Calls remain symbolic through Level-1 planning and materialise at Level-2. A planner rule that needs to look inside a transparent function may request expansion, but no rule expands speculatively.
7. Out of scope for v1 (intent — preserved here so future plans honour it):
   - **Alpha-rename of body-introduced CTEs.** v1 emits a collision diagnostic; v2 alpha-renames. (See Known Divergences.)
   - **Expansion caching keyed on `(callee_id, arg_types)`.** Salsa memoisation handles per-input caching; a dedicated cross-call expansion cache is post-v1 polish (also flagged in `gradual_typing.md`).
   - **Span-based diagnostic deduplication.** A single bad callee body emitting one frame stack per call site is intentional in v1 — see `gradual_typing.md` Known Divergences.
   - **A user-facing `Origin` API.** Origin tags are an internal contract; they have no public surface in v1.

## Known Divergences / Open Questions

- **CTE alpha-rename is deferred to v2.** Research §16 #12 commits to alpha-rename "once expansion is AST-level"; v1 ships the collision diagnostic. Once landed, the v2 path will rename body CTEs into fresh names tagged `Synthesized(fn_id, "cte-alpha-rename")`, eliminating the collision diagnostic. This is a hygiene gap, not a soundness gap — collisions surface as a diagnostic rather than as silently-wrong code.
- **`Synthesized(fn_id, reason)` `reason` taxonomy is not enumerated.** The current call sites that synthesise nodes (default-value expansion, row-variable erasure, planner strategy injection) use ad-hoc `reason` strings. A formal enumeration is post-v1.
- **Frame-trace renderer is single-level in v1** (research §16 #16). The producer populates the full stack; the LSP renderer reads only the outermost frame. Multi-level rendering is tracked as a divergence in `gradual_typing.md` and resolved by a renderer-only follow-up.
- **Provenance-aware planner passes are not yet exercised.** Until Level-2 planner work lands (`planner_integration.md` Known Divergences), there are few transformations that stress the origin-tag contract. The contract is normative now so that future passes inherit it; the test surface that pins it grows alongside the planner.
- **`CteShadowsCallerCte` covers direct caller–body collisions only.** The diagnostic fires when a model's top-level CTE name collides with a CTE declared in the body of a function the model directly calls. Transitive collisions (a function body's CTE colliding with a CTE inside another function it calls, which is itself called by the model) are not detected in v1 and are a known gap. Alpha-rename (v2) will eliminate the diagnostic entirely.
- **`column_origin` on anonymous HOF frames is producer-side only in v1.** The field is populated by the compiler when the HOF source list comes from `smelt.columns_of(t)`. The v1 LSP renderer does not read it — a dedicated renderer follow-up will surface it as an optional trailer (e.g. `(column declared at <file>:<line>)`). Until that renderer ships, the field is a no-op from the user's perspective.
- **`model_origin` on anonymous HOF frames is producer-side only in v1.** The field is populated by the compiler when the HOF source list comes from `smelt.models.with_tag(…)` or `smelt.models.all`. The v1 LSP renderer does not read it — the same renderer follow-up that surfaces `column_origin` will surface `model_origin` as an optional trailer (e.g. `(model declared at <file>:<line>)`). Until that renderer ships, the field is a no-op from the user's perspective.
- **`source_origin` on anonymous HOF frames is producer-side only in v1.** The field is populated by the compiler when the HOF source list comes from `smelt.sources.with_tag(…)` or `smelt.sources.all`. The v1 LSP renderer does not read it — the same renderer follow-up that surfaces `column_origin` will surface `source_origin` as an optional trailer (e.g. `(source declared at <file>:<line>)`). Until that renderer ships, the field is a no-op from the user's perspective.
- **`<generator>` frame for multi-model production is landed.** Generator files (per `meta_language.md` §"Multi-model production") emit an outermost `<generator>` frame at the file body's range so that diagnostics from evaluating the generator body carry a navigable provenance root. The producer-side constructor is `make_generator_frame(path, body_range)` in `crates/smelt-db/src/function_body_check.rs`; `stamp_generator_frame_onto` appends it as outermost in the frame stack. The frame is stamped on every body-evaluation diagnostic (whether or not it already carries inner HOF anonymous frames); structural file checks (`ModelDefDuplicateName`, `GeneratorBodyForbidsModelReflection`, etc.) remain frameless because they are pushed onto the diagnostic vec **after** the stamping loop completes, preserving expansion invariant 4.
- **`model_origin` on `<generator>` frames is not yet populated by the producer.** Unlike `model_origin` on anonymous HOF frames (which the producer populates per-element from the resolved `smelt.models.*` source), `model_origin` on the `<generator>` frame remains `None` in v1. Populating it would require per-diagnostic stamping (the offending `ModelDef.name` range varies per-emission while the frame is constructed once per generator file) — the current single-frame-per-file constructor cannot supply that context. A follow-up that threads the per-emission `name_span` through the frame-stamping loop is tracked in `docs/plans/20260509-meta-language-overall.md`; until then, the renderer cannot surface a per-`ModelDef.name` trailer for generator-emitted diagnostics.

## References

### Code

- `crates/smelt-types/src/signatures.rs` — `FrameInfo` (struct shape and field documentation)
- `crates/smelt-db/src/lib.rs::DiagnosticData::ExpansionFrames` — diagnostic-payload variant carrying the frame stack
- `crates/smelt-db/src/function_body_check.rs` — Tier 1 expansion (`check_smelt_fn_call`, `walk_body_with_ctx`), frame-stack push at each call (`frames.push(FrameInfo { … })`), nested-call dispatch via `NestedCallHandler`
- `crates/smelt-lsp/src/...` — `to_lsp_diagnostic` consumer that reverses the frame stack for outer-to-inner rendering

### Tests

- `crates/smelt-db/src/function_body_check.rs::tests` — single-frame and multi-frame expansion-trace coverage
- `crates/smelt-db/tests/` — workspace-level tests pinning frame ordering and `DiagnosticRelatedInformation` payload shape

### User docs

- User-visible expansion behaviour is documented under the function-error-message pages reachable from `docs-site/docs/concepts/functions.md` — to be reconciled via `/smelt:validate expansion` in a follow-up plan.

### Plans (history) — oldest → newest

- `docs/plans/20260422-smelt-functions.md` — Phases 6 (single-level frame stamping) and 12 (multi-level frames, `decl_path` / `call_site_range` data, related-information fan-out)
- `docs/plans/20260428-author-missing-specs.md` — the spec-authoring plan that produced this file

### Related specs

- `docs/specs/gradual_typing.md` — when frames are pushed, format guarantees, multi-level renderer divergence
- `docs/specs/scoping.md` — parameters-first hygiene rule, CTE-collision interaction
- `docs/specs/functions.md` — declaration grammar, `FunctionCallCycle` cycle rule (bounds expansion)
- `docs/specs/planner_integration.md` — Level-1 vs Level-2 planning; codegen-time expansion runs at L2

### Research

- `docs/research/20260413-smelt-functions.md` §16 decision 12 (Expansion mechanics: AST-level with structured provenance) — origin tags, two senses of expansion, hygiene rationale, deferral list
