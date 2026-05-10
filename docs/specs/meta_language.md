---
feature: meta_language
status: experimental
last_reviewed: 2026-05-10
owners: [andrew]
phases:
  A: spec-authored
  B: spec-authored
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

### Phase B — HOFs, lambdas, pipe, contextual reducers, `smelt.config.var`

#### Lambda syntax `fn x => body`

`fn` is a reserved keyword (lexer addition). The lambda surface is:

- `fn IDENT => EXPR` — single-argument lambda binding `IDENT` for use inside `EXPR`.
- The body `EXPR` is any meta-evaluable expression: a `smelt.<path>(...)` call, a HOF call, a pipe chain, a list literal, a record-field projection (Phase E1+), a SQL expression involving the bound name as a value or — when the bound type is a `ColumnRef` (Phase C+) — as an identifier in a splice position.
- Multi-argument lambdas (`fn (a, b) => body`) are reserved syntactically for Phase F. In Phase B, attempting to declare one produces a generic parse error (the parser does not yet recognise the form); Phase F will introduce both the CST shape and the targeted `LambdaArityNotSupported` diagnostic.
- A lambda is a value of meta-only type `Lambda<T, U>` (parameter type `T`, return type `U`). It can only be constructed in a HOF positional argument position; a lambda literal in any other position (top-level expression, named-arg value, list element, splice point, `smelt.define` argument) emits `LambdaInForbiddenPosition` at the `fn` keyword.
- A lambda cannot be assigned to a name and is never the declared sort of a `smelt.define` parameter or return type — `Lambda<T, U>` is not part of the user-writable annotation surface.

`=>` continues to mean named-argument `name => value` outside `fn` lambda bodies. The `fn` keyword resolves the parser ambiguity unambiguously: once `fn` is consumed, the immediately-following identifier is a lambda parameter and the next `=>` is the lambda arrow, regardless of surrounding context.

#### Higher-order functions

Three built-in meta-functions, called as ordinary positional calls:

| HOF | Signature | Result |
|---|---|---|
| `map` | `(xs: List<T>, f: Lambda<T, U>) -> List<U>` | new list of length `len(xs)`, element `i` is `f(xs[i])` |
| `filter` | `(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>` | sub-list of `xs` (in original order), keeping every `xs[i]` for which `p(xs[i])` is `TRUE` |
| `reduce` | `(xs: List<T>, r)` where `r` is a bare reducer identifier from the closed registry. Result sort: the reducer's declared output. | single fragment of the reducer's declared output sort |

The HOF names `map`, `filter`, `reduce` are reserved — they resolve only to the built-in HOF; a `smelt.define` declared with one of these names emits `HofNameShadowed` at the declaration. HOFs accept exactly two positional arguments and zero named arguments. The lambda's parameter type is unidirectionally inferred from the HOF's `T` per `types.md` §"Bidirectional checking"; the body is checked under that binding.

A HOF call carrying a non-`Lambda` second argument (for `map`/`filter`) emits `HofExpectsLambda`. A `reduce` call whose second argument is anything other than a bare reducer identifier from the closed registry emits `HofExpectsReducer` at the second-argument span. A lambda whose body type cannot satisfy the HOF's required result shape (e.g. `filter` requires `Lambda<T, Boolean>`) emits `LambdaResultTypeMismatch` anchored at the body expression.

#### Pipe operator `|>`

A binary operator with **first-arg pipe** semantics:

```
LHS |> f(args...)   ≡   f(LHS, args...)
```

- New token `|>` added to the lexer (distinct from `||` SQL string concatenation; the lexer must lex `||` before `|>` to avoid mis-tokenisation).
- Left-associative: `a |> b(p) |> c(q)` ≡ `c(b(a, p), q)`.
- Lower precedence than every other meta-language operator (`...`, function call, field access). Higher precedence than `;` (statement separator) — pipe never crosses a statement boundary.
- The RHS of `|>` must syntactically be a call expression (a function call: `f(args)`, `smelt.<path>(args)`, or a HOF). A non-call RHS (`x |> y`, `x |> 3 + 4`) emits `PipeRhsNotCall` at the RHS span.
- Pipe is meta-world only: both LHS and RHS evaluate at compile time. The Data-World grammar reserves no use for `|>`, so a pipe in a Data-World position (e.g. inside a `WHERE` predicate) parses and is then rejected by the splice-context check with `PipeInDataPosition`.
- A pipe expression's evaluated type and value equal the type and value of the equivalent un-piped call; pipe is purely sugar.

#### Contextual reducers

Reducers are a **closed registry of bare identifiers** reserved by the compiler. They are recognised by the type checker only as the second argument to `reduce`; everywhere else they emit `UnknownIdentifier`. A `smelt.define` declared with a reducer name emits `ReducerNameShadowed`.

| Reducer | Input | Output | Empty-list identity |
|---|---|---|---|
| `comma_sep` | `List<Expr<T>>` (any `T`) | `SelectItems<Scalar>` | empty SelectItems (elides commas at splice) |
| `and_all` | `List<Expr<Boolean>>` | `Expr<Boolean>` | `TRUE` literal |
| `or_any` | `List<Expr<Boolean>>` | `Expr<Boolean>` | `FALSE` literal |
| `union_all` | `List<TableExpr>` | `TableExpr` | none → `ReducerEmptyNoIdentity` |
| `intersect_all` | `List<TableExpr>` | `TableExpr` | none → `ReducerEmptyNoIdentity` |
| `plus_chain` | `List<Expr<Numeric>>` | `Expr<Numeric>` (LUB-promoted) | `0`-cast-to-LUB-element-type |
| `concat` | `List<Expr<Text>>` | `Expr<Text>` | empty string literal `''` |

Each reducer's empty-list identity (or its absence) is part of the closed registry's contract. Adding a reducer requires a compiler change and a spec edit — the v1 registry is not user-extensible (see Phase F's parameterised-reducer surface for the planned extension axis).

A reducer applied to a list whose element type is incompatible with the reducer's declared input emits `ReducerInputTypeMismatch` at the `reduce` argument expression. The diagnostic names the reducer and the expected vs actual element types.

#### `smelt.config.var`

`smelt.config.var(name: Text) -> Text`

A compile-time variable lookup against the workspace's `smelt.yml` `vars:` block.

- The argument must be a **literal `Text`** in Phase B; expression-valued names (`smelt.config.var(other_var)`) are reserved for Phase E1.
- The result is the variable's value rendered as `Text`. YAML scalars (`true`, `42`, `"hello"`, `null`) round-trip to the surface they would have on output (`"true"`, `"42"`, `"hello"`, `""` with a `ConfigVarNullCoercion` warning). Richer-typed reads (Boolean, Integer) land in Phase E1 with explicit schema declarations.
- Variable lookup is the only Phase B reflection surface; model / source / column reflection lands in Phases C–D.

A call to `smelt.config.var(<name>)` where `<name>` is not present in `smelt.yml` `vars:` emits `ConfigVarNotFound` at the call site. A call whose argument is not a string literal emits `ConfigVarNameNotLiteral` at the argument expression.

#### Diagnostic codes (new in Phase B)

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `LambdaInForbiddenPosition` | `fn x => body` outside a HOF positional argument | `lambda is only valid as an argument to a higher-order function` |
| `LambdaArityNotSupported` | `fn (a, b) => body` (multi-arg) — deferred to Phase F; Phase B produces a generic parse error instead | `multi-argument lambdas are not supported in v1; use a single parameter` |
| `LambdaResultTypeMismatch` | lambda body type incompatible with HOF's required result shape | `{hof} requires lambda result {expected}; found {actual}` |
| `HofExpectsLambda` | second argument to `map`/`filter` is not a `Lambda<…>` | `{hof} expects a lambda; found {actual type}` |
| `HofExpectsReducer` | second argument to `reduce` is not a registered reducer | `reduce expects a reducer; found {actual}` |
| `HofNameShadowed` | a `smelt.define` function declared with name `map`, `filter`, or `reduce` | `{name} is a reserved higher-order function name` |
| `ReducerNameShadowed` | a `smelt.define` function declared with a reducer name | `{name} is a reserved reducer name` |
| `PipeRhsNotCall` | RHS of `\|>` is not a call expression | `pipe right-hand side must be a function call` |
| `PipeInDataPosition` | a pipe expression appears in a Data-World grammar position (e.g. inside a `WHERE` predicate) | `\|> is meta-only; use SQL composition in this position` |
| `ReducerInputTypeMismatch` | reducer applied to a list whose elements don't match its declared input | `reducer {r} expects List<{T_in}>; found List<{T_actual}>` |
| `ReducerEmptyNoIdentity` | `union_all` / `intersect_all` reducing an empty list | `reducer {r} has no identity for an empty list` |
| `ConfigVarNotFound` | `smelt.config.var(<name>)` whose `<name>` is not in `smelt.yml` `vars:` | `compile-time variable {name} not declared in smelt.yml vars` |
| `ConfigVarNameNotLiteral` | `smelt.config.var` called with a non-literal-`Text` name | `smelt.config.var name must be a string literal` |
| `ConfigVarNullCoercion` | YAML `null` value coerced to empty string at a `smelt.config.var` site | `null variable {name} coerced to empty string; declare a default in smelt.yml` (warning) |

#### LSP support required by Phase B

- **Hover** on a lambda parameter inside the body shows the parameter's bound type (the HOF's `T`).
- **Hover** on a HOF call shows the result type (`List<U>` for `map`/`filter`, the reducer's output sort for `reduce`).
- **Hover** on a pipe expression shows the result type of the equivalent un-piped call.
- **Hover** on a reducer name in `reduce(_, here)` position shows the reducer's input element type, output sort, and empty-list identity (or "no identity").
- **Hover** on `smelt.config.var('x')` shows `Text` and the variable's resolved value (when present in `smelt.yml`).
- **Goto-definition** on a lambda parameter inside the body resolves to the parameter's binding occurrence in the lambda head.
- **Goto-definition** on a HOF or reducer name resolves to the built-in's reference page (`docs-site/docs/meta-language/reference.md`) by URL hint when the LSP client supports external links; otherwise no-op (graceful).
- **Goto-definition** on a `smelt.config.var('x')` argument resolves to the `vars.x:` line in `smelt.yml`.
- **Diagnostics with frame stacks**: a type error inside a lambda body carries a `Caller(span_of_hof_call)` frame plus an **anonymous frame** identifying the HOF and the source-list element index when known. The `expansion.md` Phase B spec touch registers the new anonymous-frame form (a frame with `function = "<hof>"`, `fn_id = None`, optional `element_index`).
- **Completion** inside a lambda body offers the bound parameter as the first identifier completion.
- **Completion** at the second argument position of `reduce` offers the closed reducer registry, filtered by the input list's element type when inferable.



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

#### Phase B — HOFs, lambdas, pipe, contextual reducers, `smelt.config.var`

1. **Lambda formation.** `fn x => body` constructs a `Lambda<T, U>` value where `T` is the HOF-supplied parameter type and `U` is the synthesised type of `body` under the binding `x : T`. A lambda outside a HOF positional-argument position is `LambdaInForbiddenPosition`; a lambda with multi-argument syntax is reserved for Phase F (no Phase B diagnostic — Phase B produces a generic parse error for the unrecognised form). Lambdas are values only — they have no declaration site, no name, no `smelt.<path>` reachability.

2. **Lambda parameter scoping.** Inside `body`, a bare reference to `x` resolves to the lambda parameter before any wider scope (function parameters, CTE columns, `TableExpr`-parameter columns, upstream schemas — see `scoping.md` §"Resolution order"). Lambda parameters are pushed onto the body's `TypeContext` for the duration of the body walk and popped on exit. A lambda parameter shadowing a `smelt.define` parameter or a sibling lambda parameter is permitted (lexical shadowing is the standard meaning); the inner binding wins. The `scoping.md` Phase B touch registers lambda parameters as a new scope kind.

3. **Lambda capture.** A lambda body may reference: lambda parameters in scope (the immediate parameter and any enclosing lambda parameters once multi-arg lambdas land in Phase F); the enclosing `smelt.define`'s parameters; meta-only outer-scope names (`List<T>` values, `smelt.config.var('x')` results). It must not reference SQL columns reachable only at Data-World runtime — those names do not exist at meta-evaluation time. A capture of a runtime-only name surfaces as `UnknownIdentifier` at the bare reference, with the lambda's enclosing HOF call as the diagnostic anchor.

4. **HOF evaluation.**
   - `map(xs, f)`: produces a new `List<U>` of length `len(xs)`, with element `i` equal to the result of applying `f` to `xs[i]`. Evaluation walks `xs` left-to-right exactly once; the lambda body is type-checked once (parametrically), evaluated once per element. Order is preserved.
   - `filter(xs, p)`: produces a sub-list of `xs` (in original order) keeping element `i` if and only if `p(xs[i])` evaluates to `TRUE`. The predicate body must synthesise to `Boolean`; a synthesised `Unknown` propagates per `gradual_typing.md` and the element is dropped (with no error) if the predicate is `Unknown`. Order is preserved.
   - `reduce(xs, r)`: produces a single fragment of the reducer's declared output sort. The reducer is fully resolved at type-check time from the closed registry; the second argument is a bare reducer identifier (not a value-bearing expression in v1).
   - All three HOFs require exactly two positional arguments and zero named arguments. A HOF call with named arguments emits `HofExpectsLambda` or `HofExpectsReducer` at the offending argument expression.

5. **Reducer evaluation.** `reduce(xs, r)` evaluates as follows:
   - If `xs` is non-empty, the result is the reducer's left-fold over `xs` rendered into a single fragment of the reducer's declared output sort (using the reducer's binary-operation rule: `comma_sep` produces `e1, e2, …`; `and_all` produces `e1 AND e2 AND …`; `union_all` produces `e1 UNION ALL e2 UNION ALL …`; etc.).
   - If `xs` is empty and the reducer declares an identity, the result is the identity (e.g. `and_all` → `TRUE`; `concat` → `''`).
   - If `xs` is empty and the reducer has no identity (`union_all`, `intersect_all`), the diagnostic is `ReducerEmptyNoIdentity` and the surrounding splice position emits no fragment (the position re-validates as if the `reduce` call were absent — same drop-on-error policy as `MetaSpreadInForbiddenPosition`).
   - Reducer evaluation is order-preserving (left-to-right). `union_all` and `intersect_all` produce SQL whose row order is the user's reasonable expectation given the source list's order, but smelt makes no row-order guarantee beyond what SQL itself guarantees for the underlying operator.

6. **Reducer input typing.** The reducer's declared input element type must be assignable from the actual list's element type per `types.md` §"Fragment sort subtyping" and §"Numeric promotion chain". Mismatches emit `ReducerInputTypeMismatch`. For `plus_chain` and `concat`, the output element type is the LUB of the inputs (numeric promotion or `Text` widening). For `comma_sep`, the output is `SelectItems<Scalar>` regardless of the inputs' specific `T` (the reducer collapses `List<Expr<T>>` to a select-list shape; per-element type information is preserved at the splice point but not re-exposed as a list type).

7. **Pipe desugaring.** `LHS |> CALL(args...)` is equivalent in every observable way to `CALL(LHS, args...)`. This equivalence holds for evaluated value, synthesised type, frame-stack contribution, and diagnostic anchoring. Diagnostics that would have anchored on the un-piped call argument anchor instead on the corresponding source span (LHS expression for the pipe-introduced first argument; original `args` spans for the rest). Pipe never introduces a fresh frame; the desugared call is treated as if the user had written it directly.

8. **Pipe binding.** `|>` is left-associative with the lowest meta-language precedence. `a |> b(p) |> c(q)` parses as `(a |> b(p)) |> c(q)` and desugars to `c(b(a, p), q)`. Pipe never crosses statement boundaries: `; |>` and `|> ;` are parser errors at the offending token.

9. **Pipe RHS.** The RHS must syntactically be a call expression — a function call (`f(args)`), a `smelt.<path>(args)` call, or a HOF (`map(args)`, `filter(args)`, `reduce(args)`). A non-call RHS emits `PipeRhsNotCall`; the pipe expression as a whole evaluates as if `LHS` were the un-piped result and the pipe were absent.

10. **`smelt.config.var` evaluation.** `smelt.config.var('x')` resolves at type-check time:
    - Read the workspace's `smelt.yml` `vars:` block. The path is `<workspace_root>/smelt.yml` (or per-target overlay per `smelt_yml.md`).
    - Look up `'x'` in the `vars:` map. Absent → `ConfigVarNotFound`; non-string scalar → render to `Text` per the rules below.
    - YAML scalar coercion: strings round-trip as their value (`"hello"` → `'hello'`); booleans render as `'true'` / `'false'`; integers and floats render as their decimal representation; `null` renders as `''` and warns `ConfigVarNullCoercion`.
    - The synthesised type is always `Text` in Phase B. Richer-typed reads land in Phase E1 with explicit schema declarations.
    - The argument must be a string literal in Phase B; non-literal arguments emit `ConfigVarNameNotLiteral`.

11. **HOF inline expansion frame.** When a diagnostic surfaces from inside a HOF lambda body, the diagnostic carries an **anonymous expansion frame** identifying the HOF and the source-list element index when known. The frame's shape per `expansion.md`'s extended `FrameInfo` contract: `function = "<hof name>"`, `fn_id = None` (no declaration site), `decl_path = None`, `decl_range = None`, `call_site_range = span_of(HOF call)`, plus an optional `element_index` field naming the source list index whose evaluation produced the inner error. The `expansion.md` Phase B touch registers this anonymous-frame form; until that spec touch lands, the frame is producible but the v1 LSP renderer reads only the call-site range.

12. **HOF and reducer name reservation.** The bare identifiers `map`, `filter`, `reduce`, and every entry in the closed reducer registry (`comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`) are reserved at the meta-namespace level. A `smelt.define` declaration with one of these names emits `HofNameShadowed` or `ReducerNameShadowed` at the declaration's name token. Reserved names cannot be used as `smelt.define` parameters either; a parameter named `map` emits the same diagnostic.

13. **Termination (Phase B).** All Phase B constructs terminate structurally:
    - `map` and `filter` walk a finite list once.
    - `reduce` left-folds a finite list once.
    - Pipe is purely textual desugaring at parse time.
    - `smelt.config.var` is a single map lookup.
    - Lambda bodies are checked once parametrically and evaluated once per HOF iteration; they admit no recursive HOF call against their bound parameter (a lambda body containing a HOF call is permitted, but the inner HOF's source list must come from outer scope — a lambda cannot construct a list whose iteration includes itself).

*[deferred to phases C–F]*

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

#### Phase B — why these constructs together

Phase B is the **iteration test**: the smallest slice that lets a meta `List<T>` be transformed and reduced under user control. Once Phase B ships, every subsequent phase's reflection result (`List<ColumnRef>`, `List<ModelRef>`, `List<{cohort: …}>`) plugs into the same HOF + reducer pipeline. Sequencing reflection before HOFs would force users to splice raw reflection results into SQL with only spread to compose them — the dbt scenarios in the research doc §1 require filtering and re-mapping, not just splatting.

**Why `fn x => body` rather than position-based `x => body`.** Research §4.5 lists seven candidate lambda surfaces; the contender pair was (i) position-based bare `=>` and (iv) keyword-prefixed `fn`. Position-based disambiguation has the same surface as named arguments (`name => value`); the type checker has to decide which one the user meant by looking at whether the surrounding argument is positional or named. That decision is expensive to surface in error messages ("you wrote `c => CAST(c)` in a positional argument; we read it as a lambda; did you mean a named argument named `c`?") and brittle under partial editing — flipping the surrounding argument from positional to named silently retypes the construct from lambda to named-arg. The `fn` keyword is one identifier of ceremony that locks the meaning at the token boundary; a parser sees `fn`, knows the next identifier is a lambda parameter, and any error message about a misplaced lambda anchors on `fn` itself. Multi-arg lambdas (`fn (a, b) => body`) reuse the keyword cleanly when they land in Phase F. Backups (i), (ii) `\x . body`, (iii) `|x| body`, (v) implicit `_`, (vi) parens-required `(x => body)`, and (vii) SQL-comprehension `for c in cols select …` are all rejected for reasons in research §4.5; the keyword path costs the least in user-facing error message clarity.

**Why HOFs accept exactly two positional arguments and no named arguments.** The proposed signatures are stable: `map(xs, f)`, `filter(xs, p)`, `reduce(xs, r)`. Allowing named arguments (`map(xs => cols, f => fn c => …)`) collides with the lambda surface (the second arg is a lambda value, not a named-arg form). Allowing additional positional arguments would require committing to a HOF variadic surface ahead of evidence (Phase F's `zip_with` is the closest contender, and it's a separate built-in name, not a `map` overload). Two arguments, positional only, is the smallest surface that compiles and is the surface every HOF user expects from JS, Python, OCaml, F#, Elixir, etc.

**Why a closed reducer registry (research §4.7 alt (i)).** A user-defined reducer would need to assert associativity (so the compiler can fold in any order), an identity element, and the type system tracking those properties — alternatives (ii), (iii), and (iv) in research §4.7 each require either trust-without-verification or a bigger language change (type classes / monoid instances) than the meta-plan budgets. A closed registry of seven reducers is enough for every §1 dbt-style use case (comma-separated SELECTs, AND/OR composition, table unions, numeric sums, text concatenation). The cost is that adding a reducer requires a compiler change; the benefit is that every reducer's empty-list identity is vetted and every user gets predictable semantics. Phase F's parameterised reducers (`concat_with(sep)`) extend the registry's expressive power without opening it to user definition — that is the right axis to grow when concrete pain emerges.

**Why pipe is first-arg, meta-only, and purely sugar.** Research §4.6 argues for first-arg over last-arg because HOFs naturally take their data first; matching Google Pipe SQL and DuckDB Pipe is the right ecosystem signal. Last-arg (F# style) would force every HOF signature to flip; placeholder pipe (`|>` with `_`) loses the terseness of the common case. Meta-only scope (alt a) keeps the v1 surface focused — pipe-SQL extension (alt b) is a separate paper that extends the SQL grammar and the planner, not the meta-language. Purely-sugar semantics means the type checker can desugar before checking: a pipe expression and the equivalent un-piped call have identical synthesised types, evaluation results, frame-stack contributions, and diagnostic anchoring, modulo the pipe-introduced LHS span. There is no "pipe value" type and no pipe-aware codegen; once the parser is past `|>`, the rest of the pipeline is unchanged.

**Why `smelt.config.var` is the only Phase B reflection surface.** The user-facing motivation for compile-time variables — per-environment dimension lists, threshold values, feature flags — pre-exists reflection of any kind: it is a project-level configuration concern, not a workspace-introspection concern. Shipping `smelt.config.var` in Phase B lets users write the env-conditional Phase B examples without waiting for `smelt.columns_of` (Phase C) or `smelt.models.with_tag` (Phase D). The literal-only argument restriction in Phase B is a deliberate scope cut: Phase E1's expression-valued lookups (`smelt.config.var(other_var)`) require resolving an arbitrary `Text`-valued expression at type-check time, which interacts with the loader family's determinism rules. Holding that until Phase E1 keeps Phase B's variable-lookup story self-contained.

**Why lambda formation is restricted to HOF positional arguments.** A lambda value escaping into a non-HOF position (a list element, a `smelt.define` argument, a record field) would force the meta-type system to treat `Lambda<T, U>` as a first-class member of every type's substructure — propagating the type into list inference, record fields, generic substitution, etc. None of that pays off in v1 because the only consumers of `Lambda<T, U>` are the three HOFs. Restricting lambda construction to HOF positional argument positions keeps `Lambda<T, U>` invisible to the surrounding type system; the diagnostic `LambdaInForbiddenPosition` makes the restriction concrete. When a future surface needs first-class lambdas (e.g. user-defined HOFs in a post-plan extension), the restriction is the load-bearing thing to relax.

**Why HOF and reducer names are reserved rather than overloadable.** Allowing a `smelt.define` named `map` would force the type checker to disambiguate at every call site between the built-in HOF and the user's function. The disambiguation rule would be either "user wins" (which silently retires the built-in for that workspace) or "built-in wins for two-arg calls with a lambda" (which couples disambiguation to argument shape, propagating into error messages). Reserving the names produces an immediate, anchored diagnostic at the conflicting `smelt.define` declaration. The cost is that workspaces with pre-existing functions named `map` must rename them; the benefit is that every meta-language user reads `map(xs, f)` as the same operation, regardless of workspace. The same argument applies to reducer names.

*[deferred to phases C–F; expanded as each phase lands]*

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

### Phase B invariants

- **Lambdas have no first-class surface.** `Lambda<T, U>` is a meta-only type whose values are constructed only at HOF positional argument positions and consumed only by the corresponding HOF. The type does not appear in user-writable annotations, in `smelt.define` parameters / return types, in record fields, in list elements, or in named-argument values.
- **HOFs are pure functions of their inputs.** `map`, `filter`, and `reduce` produce a result fully determined by `(xs, f|p|r)` — no clock, no random, no hidden state. Re-evaluation under the same inputs produces the same result, byte-equal at the CST level for codegen-time expansion.
- **Pipe is parser-level desugaring.** `|>` is rewritten to a call-form CST node before type checking; no type-system rule, evaluation rule, or LSP feature observes a "pipe value" — every downstream layer sees the equivalent un-piped call. Reverting a pipe to its un-piped form is a mechanical refactor and never changes semantics.
- **Reducer registry is closed.** The seven reducers in §"Contextual reducers" are the entire v1 set. Adding one requires a spec edit and a compiler change. User code may not introduce a reducer; user code may not pass an arbitrary value as `reduce`'s second argument (the second argument is parsed as a bare reducer identifier, not a value-bearing expression).
- **`smelt.config.var` is literal-only.** The argument is constrained at Phase B to be a string literal. Expression-valued lookups require Phase E1's loader-family integration and are a deliberate Phase B exclusion.
- **HOF and reducer names are workspace-wide reserved.** No `smelt.define` declaration, lambda parameter, or other meta-namespace identifier may bind these names. The reservation is part of the closed-registry contract.
- **HOF inline-expansion frames carry no `fn_id`.** The anonymous-frame form registered in `expansion.md`'s Phase B touch is the only Phase B addition to the frame-stack contract; HOF lambda bodies are not declarations and have no per-function identity. The frame's `function` field carries the HOF name; producers must populate `call_site_range` and the optional `element_index`.
- **`SmeltType::Lambda` is invariant in its parameters.** `Lambda<S, T>` and `Lambda<S', T'>` unify only when `S = S'` and `T = T'`. No subtyping rule applies. The HOF's type-checking rule binds the lambda's parameter and synthesises its return; it does not need lambda subtyping.
- **`Reducer<T>` is not a user-writable type.** The internal type-system witness for reducer identifiers is unspeced at the user surface. The v1 surface presents reducers as bare identifiers with closed-registry membership; future user-defined reducers would have to surface a `Reducer<T>` type, but that is post-plan.

### Out-of-scope by deliberate choice

- **Pipe-SQL extension** (research §4.6 alternative b) — porting the pipe operator into Data-World queries is a separate paper.
- **Tuples** — rejected in favour of records; `zip_with` (if shipped) takes a multi-arg lambda rather than producing a `List<Tuple<…>>`.
- **Generators-of-generators** — Phase E2 forbids one generator file consuming another generator's output. Cycles in workspace-shape evaluation are rejected at the spec level.
- **Heterogeneous lists / sum types** — meta lists are monomorphic. A list with mixed element types is a type error; sum types are out of scope.
- **User-defined reducers** — the v1 reducer registry is closed. Extension requires a compiler change (revisit when concrete pain emerges and a soundness-verification approach exists).
- **`infer_schema` codegen mode** — schema authoring for config loaders is required; tools that infer schemas from sample data are post-plan.

## Known Divergences / Open Questions

- **Spec is incremental.** Sections A–G are filled phase by phase. Until the corresponding phase ships, the section says `[deferred to Phase X]`. Code may not exist yet for those sections — that is the intended state.
- **Phase B code does not yet exist.** The Phase B surface and semantics in this spec are normative now (the implementation plan derives from them), but the parser additions (`fn` keyword, `|>` token, lambda CST, pipe CST), the `SmeltType::Lambda` variant, the closed reducer registry, the HOF type-inference rules, the `smelt.config.var` resolver, and every Phase B diagnostic code have not yet landed. Until the Phase B plan completes, every "Phase B" reference under §References is a target. `/smelt:validate meta_language` will report this gap until Phase B's implementation phase commits.
- **Cross-spec touches required for Phase B.** The Phase B plan must land three adjacent-spec touches alongside the implementation: (i) `expansion.md` registers the anonymous-frame form (`function = "<hof>"`, `fn_id = None`, optional `element_index`); (ii) `scoping.md` registers lambda parameters as a new scope kind (resolved before any wider scope inside the lambda body); (iii) `types.md` adds the `Lambda<T, U>` entry to its `smelt.define type annotations` enumeration with the "meta-only, not user-writable as a parameter sort" note. The meta-language spec is incomplete until all three land.
- **Phase A code does not yet exist.** The Phase A surface and semantics in this spec are normative now (the implementation plan derives from them), but `crates/smelt-parser/src/{lexer,parser,ast}.rs`, `SmeltType::List`, and the four Phase A diagnostic codes have not yet landed. Until the Phase A plan completes, every "Phase A" reference under §References is a target, not a check-pinable artifact. `/smelt:validate meta_language` will report this gap until Phase A's implementation phase commits.
- **`Array<U>(…)` runtime-array constructor surface deferred.** §Per-phase semantic rules Phase A rule 3 references the `Array<U>(…)` constructor as the explicit opt-in for the runtime-array reading of `[…]`. That constructor is Phase E2's spec increment; until then, the only Data-World path to a runtime array is the existing `[1, 2, 3]` literal in an `Expr<Array<U>>` position (governed by `types.md`).
- **Lambda surface finalised at Phase B as `fn x => body`.** Research §4.5 leaned `fn` with position-based disambiguation as backup; this spec commits to the keyword form. The disambiguation backup is not part of v1 surface.
- **HOF expansion frames are anonymous in v1.** Producers populate the `function` field with the HOF name, but the frame has no `fn_id` (HOFs are built-ins, not user-defined functions). The v1 LSP renderer reads only `call_site_range`; the per-element-index field is producer-side until a renderer follow-up surfaces it. Tracked here so future planner / renderer work preserves the contract.
- **`Reducer<T>` is not a user surface.** The closed registry presents reducers as bare identifiers; the type-system witness behind them is internal. Future user-defined reducers would require a `Reducer<T>` user-writable type and a soundness-verification approach (associativity, identity); both are post-plan.
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
  - Phase B:
    - `crates/smelt-parser/src/lexer.rs` — adds `FN` keyword and `PIPE_ARROW` (`|>`) token (lex `||` before `|>` to avoid mis-tokenisation).
    - `crates/smelt-parser/src/parser.rs` — adds `LAMBDA` and `PIPE_EXPR` productions; lowest-precedence left-associative `|>`; `RHS-must-be-call` validator producing `PipeRhsNotCall`.
    - `crates/smelt-parser/src/ast.rs` — typed wrappers for `Lambda` and `PipeExpr`.
    - `crates/smelt-types/src/signatures.rs` — `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` variant.
    - `crates/smelt-db/src/type_inference.rs` — pure inference for HOF dispatch (bidirectional binding of lambda parameter type from HOF `T`), reducer dispatch (closed-registry lookup, input-type validation, empty-list identity), pipe desugaring at AST level.
    - `crates/smelt-db/src/function_body_check.rs` — anonymous-frame stamping at HOF call sites; lambda parameter scoping in body walks.
    - `crates/smelt-db/src/lib.rs::DiagnosticCode` — every Phase B diagnostic code listed in §"Diagnostic codes (new in Phase B)".
    - `crates/smelt-db/src/lib.rs` — closed reducer registry (`REDUCER_REGISTRY`) and `smelt.config.var` resolver query against `smelt.yml` `vars:`.
    - `crates/smelt-lsp/src/lib.rs` — hover for lambdas, HOF calls, pipe expressions, reducer names, and `smelt.config.var`; goto-def for lambda parameters and `smelt.config.var` arguments; completion in lambda bodies and reducer-argument positions.
- **Tests**:
  - Phase A:
    - `crates/smelt-parser/src/{lexer,parser}.rs::tests` — token, production, and error-recovery cases.
    - `crates/smelt-db/src/type_inference.rs::tests` — list literal LUB, empty-literal target inference, spread evaluation, forbidden positions.
    - `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/meta_lists/` acceptance gate.
  - Phase B:
    - `crates/smelt-parser/src/{lexer,parser}.rs::tests` — `fn` keyword, `|>` token, lambda + pipe productions, multi-arg-lambda parser error, pipe-rhs-not-call recovery.
    - `crates/smelt-db/src/type_inference.rs::tests` — HOF dispatch, lambda parameter binding, reducer input-type checking, empty-list identity, pipe desugaring, `smelt.config.var` resolution and YAML scalar coercion.
    - `crates/smelt-db/src/function_body_check.rs::tests` — anonymous-frame stamping, multi-frame chains crossing a HOF, lambda parameter scoping under `TableExpr` parameters.
    - `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/meta_hofs/` acceptance gate.
- **User docs**:
  - Phase A: `docs-site/docs/meta-language/index.md`, `docs-site/docs/meta-language/lists.md`, `docs-site/docs/meta-language/reference.md` (alphabetical reference; populated incrementally per phase).
  - Phase B: `docs-site/docs/meta-language/hofs.md`, `docs-site/docs/meta-language/lambdas.md`, `docs-site/docs/meta-language/pipes.md`, `docs-site/docs/meta-language/reducers.md`, `docs-site/docs/meta-language/config-vars.md`; `reference.md` extended with every HOF, reducer, and `smelt.config.var`.
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
