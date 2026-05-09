---
feature: meta_language
status: experimental
last_reviewed: 2026-05-09
owners: [andrew]
phases:
  A: spec-authored
  B: deferred
  C: deferred
  D: deferred
  E1: deferred
  E2: deferred
  F: deferred
  G: deferred
---

# Meta-Language

> **What this is.** A normative spec for smelt's typed compile-time meta-language: the user-visible mechanism for constructing, transforming, and reducing lists of fragments at compile time. In scope: `List<T>`, list literals, spread operator, higher-order functions (`map` / `filter` / `reduce`), lambdas, the pipe operator `|>`, contextual reducers, reflection, records, `Map<K, V>`, and multi-model production from compile-time configuration. Out of scope: `smelt.define` function-level fragment composition (see `functions.md`); the data-world `DataType` vocabulary that meta values may eventually splice into (see `types.md`); codegen-time expansion of named functions (see `expansion.md`); resolution of names within meta-evaluated bodies (see `scoping.md`); the YAML/JSON file loader family that supplies meta-world data from disk (see `meta_config_loading.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Skeleton notice.** This spec is being filled phase by phase per `docs/plans/20260509-meta-language-overall.md`. Sections marked `[deferred to Phase X]` are placeholders — the surface they describe does not yet exist in code. The framing in §Design and §Constraints is load-bearing now (it constrains every later phase) and is filled in immediately.

## Surface

The meta-language adds compile-time-only constructs to smelt SQL files and `smelt.define` bodies. None of this surface reaches the database engine; meta evaluation happens during type checking and produces fragments that splice into Data-World SQL.

### Phase A — `List<T>`, list literals, spread

#### `List<T>` fragment-sort entry

`List<T>` is a meta-only fragment sort. `T` ranges over:

- another fragment sort (`Expr<U>`, `TableExpr`, `OrderSpec`, …);
- a `DataType` lifted as a meta literal type (`Text`, `Integer`, … — only valid in meta positions, never in Data-World annotations);
- a meta-only type introduced by a later phase (`ColumnRef`, `ModelRef`, record types);
- another `List<U>` (nesting permitted).

A `List<T>` value is **finite, ordered, immutable**. Length is fixed at construction. The runtime witness is `SmeltType::List(Box<SmeltType>)` in `crates/smelt-types/src/signatures.rs`; the corresponding Phase A entry in `types.md` §"smelt.define type annotations" enumerates the surface. `List<T>` exists only at compile time — no `List<T>` value ever reaches the database engine.

#### List literal syntax `[a, b, c]`

- Comma-separated, square-bracketed expression list.
- Trailing comma allowed (`[a, b, c,]`).
- Singleton `[x]` allowed.
- Empty `[]` allowed in a position with an inferable target sort; an `[]` in any other position emits `MetaListEmptyTypeUnknown`.
- Same surface tokens lift to either a meta `List<T>` or a Data-World `Array<U>` literal, disambiguated by target sort at type-check time. The parser produces a single `ARRAY_LITERAL` CST node; meaning is assigned by the type checker. When both the meta and Data-World readings are valid at a position, **meta wins** (see §Design — Phase A).
- Heterogeneous literals (`[1, 'hello']`) emit `MetaListHeterogeneous` (meta path) or `TypeMismatch` (Data-World path) per `types.md` §"Strict-by-default doctrine"; the inferred type is `List<Unknown>` (meta) or `Array<Unknown>` (Data).

#### Spread operator `...xs`

`...xs` is a unary prefix operator over a `List<T>` value placed in a comma-separated grammar position. The operator is **meta-only** — both the operand and the surrounding context are evaluated at compile time.

Valid positions:

- SELECT lists.
- GROUP BY clauses.
- ORDER BY clauses (with `...xs` evaluating to a `List<OrderItem>`).
- Positional argument positions of any function call (built-ins, `smelt.define`, `smelt.<path>`).
- IN-lists (`x IN (...vs)`).
- VALUES rows.
- Inside other list literals (`[a, ...xs, b]` is a single `List<T>` of length `len(xs) + 2`).

Forbidden positions (each emits `MetaSpreadInForbiddenPosition`):

- WHERE clauses (no comma-separated grammar; use the `and_all` reducer in Phase B).
- FROM clauses without an explicit reducer (no default join semantics across a `List<TableExpr>`).
- Boolean-composition contexts (`x AND ...preds`, `y OR ...preds`).
- Named-argument positions (`name => value`); spread cannot stand on the left of `=>`.

Empty-list spread elides itself and its adjacent commas: `SELECT id, ...[], created_at` ≡ `SELECT id, created_at`. Inside a list literal: `[a, ...[], b]` ≡ `[a, b]`.

`...x` where `x` is not a `List<T>` emits `MetaSpreadOnNonList`; the spread is dropped and type checking continues with the surrounding context as if the spread were absent.

#### Diagnostic codes (new in Phase A)

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `MetaListEmptyTypeUnknown` | `[]` at a position with no inferable target sort | `cannot infer element type for empty list literal` |
| `MetaListHeterogeneous` | List literal whose elements do not unify under LUB | `list elements have incompatible types: {T0}, {Tk}` |
| `MetaSpreadInForbiddenPosition` | Spread in WHERE / FROM-without-reducer / boolean / named-arg | `spread is not allowed in {position name}` |
| `MetaSpreadOnNonList` | `...x` where `x` is not a `List<T>` | `spread expects List<T>; found {actual type}` |

#### LSP support required by Phase A

- **Hover** on a list literal shows `List<T>` with `T` resolved to the inferred element type (or `Unknown` if inference failed).
- **Hover** on a spread operator shows the source list's type.
- **Goto-definition** on an identifier inside a list literal resolves via the literal — each element CST node retains its original span.
- **Diagnostics with frame stacks**: when a list literal flows into a `smelt.define` body via a parameter typed `List<T>`, errors inside the body carry a `Caller(span_of_list_literal)` frame per `expansion.md`'s frame-stack contract. Per-element provenance lands in Phase B (HOFs); Phase A only stamps the list-as-a-whole frame.

### Phase B — HOFs, lambdas, pipe, contextual reducers, `smelt.config.var` *(deferred to Phase B)*

Will define:

- Lambda syntax `fn x => body` (single-arg in v1).
- Higher-order functions: `map`, `filter`, `reduce`.
- Pipe operator `|>` (first-arg pipe).
- Contextual reducers: `comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`.
- `smelt.config.var(name)` — compile-time variable lookup from `smelt.yml` `vars:` block.

### Phase C — Narrow reflection: `smelt.columns_of`, `ColumnRef` *(deferred to Phase C)*

Will define `ColumnRef` meta record type (`name: Text`, `type: DataType`, `is_numeric: Boolean`, …) and the `smelt.columns_of(t: TableExpr) -> List<ColumnRef>` accessor.

### Phase D — Wide reflection: workspace introspection *(deferred to Phase D)*

Will define `ModelRef` and `SourceRef` meta record types and `smelt.models.*` / `smelt.sources.*` accessors (`with_tag`, listing, filtering).

### Phase E1 — Records, `Map<K,V>`, schema-typed config loaders *(deferred to Phase E1)*

Will define record meta type (inline `Record<{...}>` and named `smelt.record Name = { fields }`); `Map<K,V>` with `entries`/`keys`/`values`/`get`/`has`; and the schema-typed YAML/JSON/TOML loaders (full surface in `meta_config_loading.md`).

### Phase E2 — Multi-model production *(deferred to Phase E2)*

Will define the `generates: models` frontmatter directive; the `ModelDef` record meta type; the meta-`Text`-as-identifier lift in path positions; and the workspace-shape change ("one file may produce N models").

### Phase F — Polish *(deferred to Phase F)*

Will define parameterised reducers (e.g. `concat_with(sep)`); multi-arg lambdas; the meta-world ternary `if cond then a else b`; and `zip_with` if any shipped example demands it.

### Phase G — LSP completeness *(deferred to Phase G)*

Will define rename support for new constructs and guarantee hover/goto-def/completion/diagnostics-with-frame-stacks across every shipped meta-language surface element. (No new syntactic surface; LSP capability is part of the spec because the user-visible behaviour of editor tooling is part of "how this feature works".)

## Semantics

### Two worlds, one program

The meta-language extends smelt with a **compile-time evaluation layer**. Every program is checked and evaluated in two interlocking layers:

- **Meta-World.** Compile-time values (lists, lambdas, records, reflection results, config values). Types are fragment sorts (`Expr<T>`, `TableExpr`, `SelectItems<…>`, `OrderSpec`) plus the new types `List<T>`, `Lambda<…>`, `Record<…>`, `Map<K,V>`. Meta values exist only during compilation.
- **Data-World.** SQL the database engine sees. Types are the `DataType` vocabulary in `types.md`. Data values exist at query runtime.

The two worlds intersect at **splice points** — places where a meta value materialises into Data-World syntax. Splice points already exist (every `smelt.<path>(...)` call is one); the meta-language adds: list literal positions, spread positions, and (Phase E2) generated-model positions.

### Meta-evaluation rules (load-bearing across phases)

1. **Termination.** Meta evaluation must terminate without user-visible recursion. Lists are finite; HOFs walk once. Reflection results are bounded by workspace state. The compiler must reject any construct that admits unbounded recursion at meta level.
2. **Determinism.** Meta evaluation given the same workspace state must produce the same result. No clock, no random, no network. Environment variables are accessible only via gated APIs (Phase E1 spec touch) that opt the file out of pure determinism.
3. **Purity.** Meta evaluation has no side effects. The compiler may evaluate the same expression multiple times for caching reasons; user code may not depend on observable side effects.
4. **Single-pass type-check.** Every meta program must type-check in a single pass without execution. The type checker may invoke a (bounded) meta evaluator to compute reflection results during checking, but this evaluator is itself pure and terminates.
5. **No string templating.** Meta values are CST nodes (or values that produce CST nodes). The compiler never re-parses the output of meta evaluation.

### Per-phase semantic rules

#### Phase A — `List<T>`, list literals, spread

1. **List type formation.** A `List<T>` value is constructed only by a list literal (Phase A) or by a HOF (`map` / `filter` — Phase B). `T` is resolved at construction time and is invariant for the lifetime of the value.

2. **List literal evaluation.** `[e_1, …, e_n]` evaluates each element `e_i` in the surrounding splice context and produces a `List<T>` value of length `n`. `T` is the LUB of the element types under `types.md` §"Numeric promotion chain". A literal whose elements do not unify is `List<Unknown>` and emits `MetaListHeterogeneous`; downstream consumers of `List<Unknown>` follow the widening rule in `gradual_typing.md` §"List<Unknown> widening".

3. **Bidirectional disambiguation.** The parser produces one `ARRAY_LITERAL` CST node for `[…]`; meaning is assigned by the type checker:
   - If the surrounding target sort is `List<T>` (meta), the literal evaluates as a meta-list with element type `T`.
   - If the surrounding target sort is `Expr<Array<U>>` (Data-World) or any context that admits an array literal per the existing array-literal rules, the literal evaluates as a runtime array.
   - If both are admissible, **meta-list wins**. Users opt explicitly into the runtime-array meaning by writing the (Phase E2) `Array<U>(…)` constructor.
   - If neither is admissible, the literal is a type error at the surrounding splice position; the literal itself is `List<Unknown>`.

4. **Empty literal.** `[]` evaluates to an empty `List<T>` if and only if the surrounding context supplies a target sort; otherwise `MetaListEmptyTypeUnknown` and `List<Unknown>`. An empty `Array<U>` literal is permitted only in Data-World positions that already accept zero-length arrays.

5. **Subtyping.** `List<T>` is **covariant** in `T`. If `S <: T` per `types.md` §"Fragment sort subtyping", then `List<S> <: List<T>`. Lists are immutable, so covariance is sound.

6. **Spread evaluation.** `...xs` where `xs: List<T>` materialises into the surrounding comma-separated grammar position by emitting `n` copies of the elements at the spread's source span. Each emitted element retains a `Synthesized(SpreadFrom(span_of(xs)))` provenance origin tag (per `expansion.md` §"Provenance origin tags"). The resulting comma-separated list is then re-validated against the surrounding position's existing kind/type rules.

7. **Spread of empty list.** A spread of any compile-time empty `List<T>` emits zero copies; adjacent commas elide. Re-validation of the surrounding position then runs against the elided form.

8. **Spread position validation.** Spread in any position not enumerated under §"Spread operator" is `MetaSpreadInForbiddenPosition` and is dropped from the surrounding form (so a single misplaced spread does not avalanche follow-on errors).

9. **Spread on non-list.** `...x` where `x` is not a `List<T>` is `MetaSpreadOnNonList`. The spread is dropped; the surrounding position type-checks as if the spread were absent.

10. **Compile-time-only.** No `List<T>` value reaches the database engine. After meta-evaluation, the Data-World CST handed to codegen contains no `ARRAY_LITERAL` and no spread node; every list value has been consumed by spread, by a HOF (Phase B), by a reducer (Phase B), or by a record / map / generator (Phase E1+).

11. **Termination.** Phase A introduces no meta-recursion. List literal evaluation walks the elements left-to-right exactly once; spread walks the source list exactly once. Wall-clock cost is O(n) in the source length.

*[deferred to phases B–F]*

## Design

### Why a meta-language at all

`smelt.define` (`functions.md`) closed the gap on **fragment-level** reuse — predicates, expressions, table transformers, select-list shapes can be parameterised, called by name, and inlined with full type checking. It does not address the class of dbt patterns where the *input to the SQL is computed from the project itself*: union all models matching a tag, coalesce all numeric columns, generate one staging model per source-table entry in a YAML file. These patterns require iterating over compile-time data — a list of models, a list of columns, a list of config rows — and reducing the result into a SQL fragment that splices into a model.

dbt does this through Jinja string-substitution. That choice forfeits typing, navigation, and LSP feedback inside macros: `{{ col.name }}` resolves to nothing, errors anchor to the post-substitution SQL, refactoring is text-only. smelt's meta-language proposes to give every meta value a type, every cross-reference an LSP-resolvable definition, every diagnostic a source span. The unifying claim, restated from the research doc: smelt already has a meta-world (fragment sorts, two-tiered expansion, splice contexts); making it user-visible is a layering exercise, not a new language inside the language.

The full design rationale, alternatives considered at every level (lambda surface, reducer registry, list literal disambiguation, reflection API shape, multi-model production mechanism), and the framing of the meta-/data-world boundary live in `docs/research/20260507-typed-meta-programming.md`. This spec records the decisions; the research doc records why they look like this.

### Why a single spec rather than per-construct specs

The constructs are deeply interdependent. Lambdas have no use without HOFs; HOFs have no use without lists; lists have limited use without reflection; reflection has limited use without records; records have limited use without loaders. Splitting this into seven specs would force every later spec to repeat the framing of the meta-/data-world boundary and the meta-evaluation rules above. One spec, one framing, multiple Surface entries that grow phase by phase.

The exception is `meta_config_loading.md` — the file-loading family is large enough (formats, schema authoring, validation diagnostics, per-target overlay) to warrant its own spec, with this one referencing it.

### Per-phase design rationale

#### Phase A — why these three constructs first

Phase A ships the **lifting test** — the smallest slice that exercises the meta-/data-world boundary without committing to HOFs, lambdas, or reflection. Every later phase plugs into the type-checker hooks Phase A introduces; if the lift mechanism is wrong, Phase B onwards is unsalvageable.

**Why one parameterised `List<T>` rather than per-use list types.** The closest existing type is `SmeltType::SelectItems { kind, context }`, but it is contextually constrained (carries an `ExprKind` ceiling and a context binding) and only appears at SELECT-list splice points. A user writing `union by tag` (Phase D) wants a `List<TableExpr>`; a user writing `coalesce(*numerics)` (Phase C) wants a `List<Expr<Numeric>>`; a generator (Phase E2) wants a `List<ModelDef>`. Research §4.1 alternative (ii) — `ExprList<T>`, `TableList`, `OrderList` — was rejected because it forces a new type per use case, none of which compose. `List<T>` is the smallest type-theoretic addition that handles every Phase A–E2 demand. The existing `SelectItems<…>` is preserved (Phase A does not retire it); the two coexist, and Phase B specifies exactly when `List<Expr<T>>` may be used where `SelectItems<Scalar>` is expected.

**Why bidirectional disambiguation rather than a distinct sigil.** Research §4.2 alternative (ii) — `${...}` / `meta[...]` / `(| … |)` — was rejected because users would have two surface forms for similar things. Bidirectional checking is already pervasive in smelt (numeric promotion, `Concrete(T)` resolution, row-variable binding); adding one more bidirectional rule is in-character. The cost — non-local meaning during partial editing — is real; the LSP mitigates it by showing "literal accepted in two contexts; current context expects `List<T>` / `Array<U>`" on hover. Function-style constructors (`list(a, b, c)`) were rejected because the bare name `list` is a likely user identifier and the variadic constructor reads worse than `[…]` for long lists.

**Why "meta-list wins" when both readings are valid.** The only Data-World position that genuinely admits both meanings today is the `Expr<Array<U>>` slot. Defaulting to meta keeps Phase A users (who are here to write meta code; the alternative does not yet exist) on the path that motivates the work. Once Phase E2 ships `Array<U>(…)`, users who want the runtime array opt in explicitly; the implicit lift remains meta-first. The reverse default (Data wins) would force every meta user to type-annotate the call site to suppress the array reading, which is exactly the kind of ceremony §"Why a meta-language at all" rejects.

**Why `...xs` rather than always-explicit reducers.** Research §4.3 alternative (iii) — every reduction is a `comma_sep(xs)` / `union_all(xs)` call — was rejected because the common case (splat into a comma-separated grammar position) reads worst when stripped of the spread sugar. Spread keeps the common case terse; reducers (Phase B) remain available for boolean composition, table-set composition, and expression-tree composition where there is no default reduction. `*xs` (Python style) was rejected because `*` is heavily SQL-loaded (`SELECT *`, multiplication); `...` is currently unused in smelt's grammar and a one-token lookahead distinguishes it from any malformed identifier.

**Why covariant subtyping.** `List<T>` is immutable in this language; the standard objection to covariance — a write through the supertype writes a wrong-typed value — does not apply. Mainstream typed languages (Java's wildcards, Scala's `List`, Kotlin's `List`) all expose covariant immutable lists. Variance policed at the spec level keeps the LUB rules sound and matches what users expect from immutable containers. Invariance was considered and rejected because it forces every use site to call `map(_, identity)` to widen, which is friction with no payback.

**Why the empty-list rules are bidirectional and not "always inferable".** A `[]` whose target type cannot be inferred from context is a type error at Phase A, not a `List<Bottom>` / `List<Nothing>` placeholder. Adding `Bottom` to the meta-type vocabulary is a load-bearing decision that benefits no Phase A example; introducing it would propagate into every later phase's LUB rules without any user-visible payback. The diagnostic `MetaListEmptyTypeUnknown` is the simpler answer: tell the user, suggest a target-typed annotation, move on.

*[deferred to phases B–F; expanded as each phase lands]*

## Constraints & Invariants

### Meta-world invariants (always hold)

- **Meta evaluation never reaches the database engine.** Every meta value is consumed during type checking or codegen-time expansion. The DB-facing SQL contains only Data-World constructs.
- **`smelt-db/src/type_inference.rs` remains pure.** New HOF and reflection rules are added as pure functions; Salsa queries call them, they do not call Salsa queries. (See `CLAUDE.md` Pure Function Rule.)
- **Termination is structural, not check-and-error.** The grammar admits no construct that requires runtime fixed-point iteration. If a syntax extension would admit unbounded recursion, it is rejected at the spec level, not allowed and policed at evaluation time.
- **No expansion-frame regression.** Adding HOF and multi-model production must not weaken the `expansion.md` frame-stack contract. Diagnostics from inside a `map(xs, fn x => …)` body must surface with a frame stack that names `map` and the per-element index.
- **Bidirectional checking remains decidable.** New types interlock with existing widening rules without introducing non-deterministic checks.

### Phase A invariants

- **Lists are immutable.** Phase A introduces no mutation operation. Phase B's `map` / `filter` produce new `List<T>` values; the originals are unobservable as mutated.
- **Lists are finite.** Length is known at the moment of construction. Phase A admits no streaming, lazy, or infinite-list construct.
- **`SmeltType::List(Box<SmeltType>)` is the canonical meta-list witness.** The existing `SmeltType::SelectItems { kind, context }` does not become `List<…>` and is not retired in Phase A. The two coexist; `SelectItems` remains the splice-context-bearing form for SELECT lists. The `List<T>` ↔ `SelectItems<…>` bridge is Phase B territory.
- **Phase A admits no implicit meta-to-data lift.** The spread operator passes meta-list elements into Data-World grammar slots without changing their kind; meta-`Text` does not lift to a SQL identifier in Phase A (that lift is Phase E2 and is enumerated narrowly by grammar slot).
- **`...` token is exclusive to spread.** Phase A reserves `...` in the lexer; it is not used by any other grammar construct. Future phases may extend its use only within the spread family (e.g. row-tail markers `..r` in `Struct<{…}>` are a separate token spelled with two dots, already in use).

### Out-of-scope by deliberate choice

- **Pipe-SQL extension** (research §4.6 alternative b) — porting the pipe operator into Data-World queries is a separate paper.
- **Tuples** — rejected in favour of records; `zip_with` (if shipped) takes a multi-arg lambda rather than producing a `List<Tuple<…>>`.
- **Generators-of-generators** — Phase E2 forbids one generator file consuming another generator's output. Cycles in workspace-shape evaluation are rejected at the spec level.
- **Heterogeneous lists / sum types** — meta lists are monomorphic. A list with mixed element types is a type error; sum types are out of scope.
- **User-defined reducers** — the v1 reducer registry is closed. Extension requires a compiler change (revisit when concrete pain emerges and a soundness-verification approach exists).
- **`infer_schema` codegen mode** — schema authoring for config loaders is required; tools that infer schemas from sample data are post-plan.

## Known Divergences / Open Questions

- **Spec is incremental.** Sections A–G are filled phase by phase. Until the corresponding phase ships, the section says `[deferred to Phase X]`. Code may not exist yet for those sections — that is the intended state.
- **Phase A code does not yet exist.** The Phase A surface and semantics in this spec are normative now (the implementation plan derives from them), but `crates/smelt-parser/src/{lexer,parser,ast}.rs`, `SmeltType::List`, and the four Phase A diagnostic codes have not yet landed. Until the Phase A plan completes, every "Phase A" reference under §References is a target, not a check-pinable artifact. `/smelt:validate meta_language` will report this gap until Phase A's implementation phase commits.
- **`Array<U>(…)` runtime-array constructor surface deferred.** §Per-phase semantic rules Phase A rule 3 references the `Array<U>(…)` constructor as the explicit opt-in for the runtime-array reading of `[…]`. That constructor is Phase E2's spec increment; until then, the only Data-World path to a runtime array is the existing `[1, 2, 3]` literal in an `Expr<Array<U>>` position (governed by `types.md`).
- **Lambda surface — `fn x => body` keyword vs. positional disambiguation.** Research §4.5 leans `fn`, with positional disambiguation as backup. Phase B finalises the choice; until then the spec describes the leaned shape only.
- **Reflection API namespace organisation.** Research §4.8 lands on functional accessors under `smelt.*`. The exact namespace layout (`smelt.models` vs `smelt.workspace.models`) is a Phase D decision.
- **Multi-model production mechanism.** Research §4.10.4 leans on a frontmatter directive (`generates: models`) plus a body returning `List<ModelDef>`. Phase E2 finalises; the meta-plan §10 stop-the-line condition catches scope expansion.
- **Meta-`Text`-as-identifier lift narrowness rule.** Phase E2 must commit to which SQL grammar slots admit a meta-`Text` lift (model path components, column aliases, CTE names) and which do not (arbitrary keywords). The narrowness is what keeps the lift from drifting into Jinja territory.

## References

- **Code** *(populated as phases land — Phase A entries are aspirational targets until the implementation plan lands; `/smelt:validate` will pin them once code exists)*:
  - Phase A:
    - `crates/smelt-parser/src/lexer.rs` — adds `LBRACKET`, `RBRACKET`, `DOTDOTDOT` tokens.
    - `crates/smelt-parser/src/parser.rs` — adds `ARRAY_LITERAL` (reused for list literals) and `LIST_SPREAD` productions.
    - `crates/smelt-parser/src/ast.rs` — typed wrappers for the new CST nodes.
    - `crates/smelt-types/src/signatures.rs` — `SmeltType::List(Box<SmeltType>)` variant.
    - `crates/smelt-db/src/type_inference.rs` — pure inference for list literals and spread (LUB, covariant subtyping, empty-literal handling).
    - `crates/smelt-db/src/lib.rs::DiagnosticCode` — `MetaListEmptyTypeUnknown`, `MetaListHeterogeneous`, `MetaSpreadInForbiddenPosition`, `MetaSpreadOnNonList`.
    - `crates/smelt-lsp/src/lib.rs` — hover for list/spread.
- **Tests**:
  - Phase A:
    - `crates/smelt-parser/src/{lexer,parser}.rs::tests` — token, production, and error-recovery cases.
    - `crates/smelt-db/src/type_inference.rs::tests` — list literal LUB, empty-literal target inference, spread evaluation, forbidden positions.
    - `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/meta_lists/` acceptance gate.
- **User docs**:
  - Phase A: `docs-site/docs/meta-language/index.md`, `docs-site/docs/meta-language/lists.md`, `docs-site/docs/meta-language/reference.md` (alphabetical reference; populated incrementally per phase).
- **Plans (history)**:
  - `docs/plans/20260509-meta-language-overall.md` — meta-plan / phase status table
  - `docs/plans/20260509-meta-language-A.md` — Phase A *(when written)*
  - `docs/plans/20260509-meta-language-B.md` — Phase B *(when written)*
  - `docs/plans/20260509-meta-language-C.md` — Phase C *(when written)*
  - `docs/plans/20260509-meta-language-D.md` — Phase D *(when written)*
  - `docs/plans/20260509-meta-language-E1.md` — Phase E1 *(when written)*
  - `docs/plans/20260509-meta-language-E2.md` — Phase E2 *(when written)*
  - `docs/plans/20260509-meta-language-F.md` — Phase F *(when written)*
  - `docs/plans/20260509-meta-language-G.md` — Phase G *(when written)*
- **Related specs**:
  - `docs/specs/functions.md` — `smelt.define`, fragment sorts, named arguments (parser disambiguation surface)
  - `docs/specs/types.md` — `DataType` vocabulary, fragment-sort grammar, strict-by-default doctrine
  - `docs/specs/expansion.md` — codegen-time expansion, frame stacks, `Caller`/`Callee`/`Synthesized` provenance — extended by Phase B (HOFs) and Phase E2 (multi-model)
  - `docs/specs/scoping.md` — body scoping, splice contexts, parameters-first; lambda parameter scoping (Phase B) and generator-file scoping (Phase E2) plug in here
  - `docs/specs/architecture.md` — `smelt.<path>` resolution, project layout — Phase E2 amends the "1 file = 1+ models" invariant
  - `docs/specs/gradual_typing.md` — `Unknown` widening — Phase A spec touch documents `List<Unknown>` rules
  - `docs/specs/meta_config_loading.md` — file-loader family for `smelt.config.load_yaml` etc. (Phase E1)
  - `docs/specs/model_selection.md`, `incremental_models.md`, `python_models.md`, `data_catalog.md`, `schema_evolution.md`, `cli.md`, `datagen.md` — Phase E2 cross-feature touches (see meta-plan §6)
  - `docs/specs/lsp.md` — LSP support obligations per phase
- **Research**:
  - `docs/research/20260507-typed-meta-programming.md` — design oracle: framing, alternatives at every choice point, sequencing, worked examples, open questions
  - `docs/research/20260413-smelt-functions.md` — parent paper for fragment sorts and `smelt.define`
