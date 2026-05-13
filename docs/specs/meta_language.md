---
feature: meta_language
status: experimental
last_reviewed: 2026-05-13
owners: [andrew]
---

# Meta-Language

> **What this is.** A normative spec for smelt's typed compile-time meta-language: the user-visible mechanism for constructing, transforming, and reducing lists of fragments at compile time. In scope: `List<T>`, list literals, spread operator, higher-order functions (`map` / `filter` / `reduce`), lambdas, the pipe operator `|>`, contextual reducers, reflection, records, `Map<K, V>`, and multi-model production from compile-time configuration. Out of scope: `smelt.define` function-level fragment composition (see `functions.md`); the data-world `DataType` vocabulary that meta values may eventually splice into (see `types.md`); codegen-time expansion of named functions (see `expansion.md`); resolution of names within meta-evaluated bodies (see `scoping.md`); the YAML/JSON file loader family that supplies meta-world data from disk (see `meta_config_loading.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.

## Surface

The meta-language adds compile-time-only constructs to smelt SQL files and `smelt.define` bodies. None of this surface reaches the database engine; meta evaluation happens during type checking and produces fragments that splice into Data-World SQL.

### Lists and spread

#### `List<T>` fragment-sort entry

`List<T>` is a meta-only fragment sort. `T` ranges over:

- another fragment sort (`Expr<U>`, `TableExpr`, `OrderSpec`, …);
- a `DataType` lifted as a meta literal type (`Text`, `Integer`, … — only valid in meta positions, never in Data-World annotations);
- a meta-only type introduced for reflection or records (`ColumnRef`, `ModelRef`, record types);
- another `List<U>` (nesting permitted).

A `List<T>` value is **finite, ordered, immutable**. Length is fixed at construction. The runtime witness is `SmeltType::List(Box<SmeltType>)` in `crates/smelt-types/src/signatures.rs`; the corresponding entry in `types.md` §"smelt.define type annotations" enumerates the surface. `List<T>` exists only at compile time — no `List<T>` value ever reaches the database engine.

#### List literal syntax `[a, b, c]`

- Comma-separated, square-bracketed expression list.
- Trailing comma allowed (`[a, b, c,]`).
- Singleton `[x]` allowed.
- Empty `[]` allowed in a position with an inferable target sort; an `[]` in any other position emits `MetaListEmptyTypeUnknown`.
- Same surface tokens lift to either a meta `List<T>` or a Data-World `Array<U>` literal, disambiguated by target sort at type-check time. The parser produces a single `ARRAY_LITERAL` CST node; meaning is assigned by the type checker. When both the meta and Data-World readings are valid at a position, **meta wins** (see §Design — Lists and spread).
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

- WHERE clauses (no comma-separated grammar; use the `and_all` reducer).
- FROM clauses without an explicit reducer (no default join semantics across a `List<TableExpr>`).
- Boolean-composition contexts (`x AND ...preds`, `y OR ...preds`).
- Named-argument positions (`name => value`); spread cannot stand on the left of `=>`.

Empty-list spread elides itself and its adjacent commas: `SELECT id, ...[], created_at` ≡ `SELECT id, created_at`. Inside a list literal: `[a, ...[], b]` ≡ `[a, b]`.

`...x` where `x` is not a `List<T>` emits `MetaSpreadOnNonList`; the spread is dropped and type checking continues with the surrounding context as if the spread were absent.

#### List and spread diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `MetaListEmptyTypeUnknown` | `[]` at a position with no inferable target sort | `cannot infer element type for empty list literal` |
| `MetaListHeterogeneous` | List literal whose elements do not unify under LUB | `list elements have incompatible types: {T0}, {Tk}` |
| `MetaSpreadInForbiddenPosition` | Spread in WHERE / FROM-without-reducer / boolean / named-arg | `spread is not allowed in {position name}` |
| `MetaSpreadOnNonList` | `...x` where `x` is not a `List<T>` | `spread expects List<T>; found {actual type}` |

#### LSP support for lists and spread

- **Hover** on a list literal shows `List<T>` with `T` resolved to the inferred element type (or `Unknown` if inference failed).
- **Hover** on a spread operator shows the source list's type.
- **Goto-definition** on an identifier inside a list literal resolves via the literal — each element CST node retains its original span.
- **Diagnostics with frame stacks**: when a list literal flows into a `smelt.define` body via a parameter typed `List<T>`, errors inside the body carry a `Caller(span_of_list_literal)` frame per `expansion.md`'s frame-stack contract. Per-element provenance is contributed by HOFs (see §"Lambdas and higher-order functions"); the list-as-a-whole frame is stamped at every list-literal flow.

### Lambdas and higher-order functions

#### Lambda syntax `fn x => body`

`fn` is a reserved keyword (lexer addition). The lambda surface is:

- `fn IDENT => EXPR` — single-argument lambda binding `IDENT` for use inside `EXPR`.
- The body `EXPR` is any meta-evaluable expression: a `smelt.<path>(...)` call, a HOF call, a pipe chain, a list literal, a record-field projection, a SQL expression involving the bound name as a value or — when the bound type is a `ColumnRef` — as an identifier in a splice position.
- Multi-argument lambdas (`fn (a, b) => body`) are reserved syntactically; an attempt to declare one emits `LambdaArityNotSupported` at the parameter list. Detection runs as a text shape check on the HOF's second argument.
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

#### Lambda and HOF diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `LambdaInForbiddenPosition` | `fn x => body` outside a HOF positional argument | `lambda is only valid as an argument to a higher-order function` |
| `LambdaArityNotSupported` | `fn (a, b) => body` (multi-arg) | `multi-argument lambdas are not supported; use a single parameter` |
| `LambdaResultTypeMismatch` | lambda body type incompatible with HOF's required result shape | `{hof} requires lambda result {expected}; found {actual}` |
| `HofExpectsLambda` | second argument to `map`/`filter` is not a `Lambda<…>` | `{hof} expects a lambda; found {actual type}` |
| `HofExpectsReducer` | second argument to `reduce` is not a registered reducer | `reduce expects a reducer; found {actual}` |
| `HofNameShadowed` | a `smelt.define` function declared with name `map`, `filter`, or `reduce` | `{name} is a reserved higher-order function name` |

#### LSP support for lambdas and HOFs

- **Hover** on a lambda parameter inside the body shows the parameter's bound type (the HOF's `T`).
- **Hover** on a HOF call shows the result type (`List<U>` for `map`/`filter`, the reducer's output sort for `reduce`).
- **Goto-definition** on a lambda parameter inside the body resolves to the parameter's binding occurrence in the lambda head.
- **Goto-definition** on a HOF name resolves to the built-in's reference page (`docs-site/docs/meta-language/reference.md`) by URL hint when the LSP client supports external links; otherwise no-op (graceful).
- **Diagnostics with frame stacks**: a type error inside a lambda body carries a `Caller(span_of_hof_call)` frame plus an **anonymous frame** identifying the HOF and the source-list element index when known. The `expansion.md` anonymous-frame contract registers this form (a frame with `function = "<hof>"`, `fn_id = None`, optional `element_index`).
- **Completion** inside a lambda body offers the bound parameter as the first identifier completion.

### Pipe operator

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

#### Pipe diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `PipeRhsNotCall` | RHS of `\|>` is not a call expression | `pipe right-hand side must be a function call` |
| `PipeInDataPosition` | a pipe expression appears in a Data-World grammar position (e.g. inside a `WHERE` predicate) | `\|> is meta-only; use SQL composition in this position` |

#### LSP support for pipe

- **Hover** on a pipe expression shows the result type of the equivalent un-piped call.

### Contextual reducers

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

Each reducer's empty-list identity (or its absence) is part of the closed registry's contract. Adding a reducer requires a compiler change and a spec edit — the reducer registry is not user-extensible.

A reducer applied to a list whose element type is incompatible with the reducer's declared input emits `ReducerInputTypeMismatch` at the `reduce` argument expression. The diagnostic names the reducer and the expected vs actual element types.

#### Reducer diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `ReducerNameShadowed` | a `smelt.define` function declared with a reducer name | `{name} is a reserved reducer name` |
| `ReducerInputTypeMismatch` | reducer applied to a list whose elements don't match its declared input | `reducer {r} expects List<{T_in}>; found List<{T_actual}>` |
| `ReducerEmptyNoIdentity` | `union_all` / `intersect_all` reducing an empty list | `reducer {r} has no identity for an empty list` |

#### LSP support for reducers

- **Hover** on a reducer name in `reduce(_, here)` position shows the reducer's input element type, output sort, and empty-list identity (or "no identity").
- **Goto-definition** on a reducer name resolves to the built-in's reference page (`docs-site/docs/meta-language/reference.md`) by URL hint when the LSP client supports external links; otherwise no-op (graceful).
- **Completion** at the second argument position of `reduce` offers the closed reducer registry, filtered by the input list's element type when inferable.

### Compile-time variables

`smelt.config.var(name: Text) -> Text`

A compile-time variable lookup against the workspace's `smelt.yml` `vars:` block.

- The argument must be a **literal `Text`**; expression-valued names (`smelt.config.var(other_var)`) are not yet supported.
- The result is the variable's value rendered as `Text`. YAML scalars (`true`, `42`, `"hello"`, `null`) round-trip to the surface they would have on output (`"true"`, `"42"`, `"hello"`, `""` with a `ConfigVarNullCoercion` warning). Richer-typed reads (Boolean, Integer) require explicit schema declarations.
- Variable lookup is the only non-reflection workspace-state surface in this category; model / source / column reflection lives under §"Reflection" below.

A call to `smelt.config.var(<name>)` where `<name>` is not present in `smelt.yml` `vars:` emits `ConfigVarNotFound` at the call site. A call whose argument is not a string literal emits `ConfigVarNameNotLiteral` at the argument expression.

#### Compile-time variable diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `ConfigVarNotFound` | `smelt.config.var(<name>)` whose `<name>` is not in `smelt.yml` `vars:` | `compile-time variable {name} not declared in smelt.yml vars` |
| `ConfigVarNameNotLiteral` | `smelt.config.var` called with a non-literal-`Text` name | `smelt.config.var name must be a string literal` |
| `ConfigVarNullCoercion` | YAML `null` value coerced to empty string at a `smelt.config.var` site | `null variable {name} coerced to empty string; declare a default in smelt.yml` (warning) |

#### LSP support for compile-time variables

- **Hover** on `smelt.config.var('x')` shows `Text` and the variable's resolved value (when present in `smelt.yml`).
- **Goto-definition** on a `smelt.config.var('x')` argument resolves to the `vars.x:` line in `smelt.yml`.

### Reflection: `smelt.columns_of` and `ColumnRef`

#### `smelt.columns_of` accessor

`smelt.columns_of(t: TableExpr) -> List<ColumnRef>`

A meta-only accessor that returns the column list of a `TableExpr`-valued meta value. The argument may be:

- A `smelt.<path>` reference resolving to a model, source, or seed (the existing schema-resolution machinery in `crates/smelt-db/src/schema.rs` supplies the `ModelSchema`).
- A `smelt.define` parameter declared `TableExpr` or `TableExpr<{…}>`. At body-check time the result type is `List<ColumnRef>` parametrically; at expansion time at each call site the concrete schema is bound and the list is materialised.
- The result of any other `TableExpr`-typed expression resolved through prior expansion (a CTE alias, a subquery alias).

`smelt.columns_of` is called as an ordinary positional function with exactly one argument; named arguments emit `ColumnsOfNamedArgument` at the named-arg span. The single positional argument's evaluated type must be assignable to `TableExpr`; mismatches emit `ColumnsOfRequiresTableExpr` at the argument expression. A `TableExpr` whose schema cannot be statically resolved at expansion time emits `ColumnsOfUnresolvableSchema` at the call and the surrounding HOF call drops its splice without further diagnostics (same drop-on-error policy as `MetaSpreadInForbiddenPosition`).

#### `ColumnRef` meta record type

`ColumnRef` is a closed meta-only record type with three fields:

| Field | Type | Meaning |
|---|---|---|
| `name` | `Text` | The column's identifier as it appears in the source schema (un-quoted; case-preserved) |
| `type` | `DataType` (meta literal) | The column's `DataType` from `types.md` §"`DataType` vocabulary" |
| `is_numeric` | `Boolean` | `TRUE` iff `type` is in the `Numeric` constraint set per `types.md` §"Type constraints" |

Field access uses dot-notation (`c.name`, `c.type`, `c.is_numeric`). Field access on any other identifier emits `ColumnRefFieldUnknown` at the field span. `ColumnRef` is **closed**: the field set is exactly these three fields. Adding a field requires a spec edit and a compiler change; the registry pattern matches the closed reducer registry.

`ColumnRef` is meta-only. It is not user-writable as a `smelt.define` parameter or return type, not a list element type users construct in literals, and not a value that reaches the database engine. The internal `SmeltType` witness behind `ColumnRef` is unspeced at the user surface; users never write `Record<{name: Text, type: DataType, is_numeric: Boolean}>` and never need to. The user-writable record surface (`smelt.record Name = { … }`) is a separate construct that does not retroactively expose `ColumnRef`'s structure.

#### Reflection diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|------|------|---------------|
| `ColumnsOfRequiresTableExpr` | `smelt.columns_of(x)` whose `x` synthesises to a type not assignable to `TableExpr` | `smelt.columns_of expects TableExpr; found {actual}` |
| `ColumnsOfNamedArgument` | `smelt.columns_of` called with a named argument | `smelt.columns_of takes one positional argument; named arguments are not supported` |
| `ColumnsOfUnresolvableSchema` | At expansion time, `smelt.columns_of(t)` whose `t` resolves to an `Unknown` schema | `cannot resolve column list for {t}; upstream schema is unknown` |
| `ColumnRefFieldUnknown` | Field access on a `ColumnRef` value with an identifier outside the closed field set | `ColumnRef has no field {name}; expected one of: name, type, is_numeric` |

#### LSP support for reflection

- **Hover** on `smelt.columns_of(t)` shows `List<ColumnRef>` and (when `t`'s schema is statically resolvable) the resolved column count plus the first five column names.
- **Hover** on a `ColumnRef`-typed binding (a lambda parameter inside a `columns_of` HOF chain) shows `ColumnRef` plus the closed field list with each field's type.
- **Hover** on a field projection `c.name` / `c.type` / `c.is_numeric` shows the field's declared type (and, when the projection is reached at expansion time over a resolvable list, its concrete value at the current call site).
- **Goto-definition** on `smelt.columns_of` resolves to the reference page (URL hint, graceful no-op when the client lacks support).
- **Completion** at a field-projection site (`c.<cursor>`) offers the closed field list (`name`, `type`, `is_numeric`).
- **Completion** at a `smelt.columns_of(<cursor>)` argument position offers in-scope `TableExpr`-valued names (`smelt.<path>` references and the enclosing function's `TableExpr` parameters).
- **Diagnostics with frame stacks**: a type error inside a HOF lambda body whose source list comes from `smelt.columns_of(t)` carries the anonymous frame plus an optional `column_origin` field on the per-element entry, recording the source column's declaration span when statically traceable. The `expansion.md` anonymous-frame contract registers this extension.

### Reflection: `smelt.models`, `smelt.sources`, `ModelRef`, `SourceRef`

#### `smelt.models` accessors

| Accessor | Signature | Returns |
|---|---|---|
| `smelt.models.with_tag` | `smelt.models.with_tag(tag: Text) -> List<ModelRef>` | Every model whose merged tag set (frontmatter `tags:` ∪ `smelt.yml` `models.<name>.tags`, deduplicated per `crates/smelt-core/src/config.rs::Config::get_tags`) contains `tag`, sorted ascending by `path`. |
| `smelt.models.all` | `smelt.models.all() -> List<ModelRef>` | Every model in the workspace, sorted ascending by `path`. |

`smelt.models` is a closed accessor namespace; the set is exactly the two accessors above. A reference to an unknown accessor (`smelt.models.bogus()`) emits `WideReflectionUnknownAccessor` at the accessor name span. Named arguments to `with_tag` emit `WithTagNamedArgument` at the named-argument span. An argument to `with_tag` whose evaluated type is not assignable to compile-time `Text` emits `WithTagRequiresText` at the argument expression. The argument's value must be a compile-time-resolvable `Text` (string literal, `smelt.config.var(...)` result, or any meta-`Text` expression); a runtime `Expr<Text>` argument emits `WithTagRequiresText`. `smelt.models.all` accepts no arguments; any positional or named argument emits `WideReflectionUnexpectedArgument` at the offending argument's span.

#### `smelt.sources` accessors

| Accessor | Signature | Returns |
|---|---|---|
| `smelt.sources.with_tag` | `smelt.sources.with_tag(tag: Text) -> List<SourceRef>` | Every source whose declared `tags:` set (from the source's YAML declaration per `crates/smelt-core/src/config.rs`) contains `tag`, sorted ascending by `path`. |
| `smelt.sources.all` | `smelt.sources.all() -> List<SourceRef>` | Every source in the workspace, sorted ascending by `path`. |

`smelt.sources` is a closed accessor namespace with the same disposition as `smelt.models`. Diagnostic codes are shared (`WithTagRequiresText`, `WithTagNamedArgument`, `WideReflectionUnknownAccessor`, `WideReflectionUnexpectedArgument`); the message text substitutes "sources" for "models" where appropriate.

#### `ModelRef` meta record type

`ModelRef` is a closed meta-only record type with four fields:

| Field | Type | Meaning |
|---|---|---|
| `path` | `Text` | Workspace-relative file path of the model's source file, normalised with `/` separators (independent of host OS). |
| `name` | `Text` | The model's `smelt.<path>` identifier — the final path segment without the `.sql` suffix. |
| `tags` | `List<Text>` | The model's merged tag set, in the deduplication order produced by `Config::get_tags` (`smelt.yml` tags first, then SQL frontmatter `tags:` entries not already present). |
| `columns` | `List<ColumnRef>` | The model's column list. Equivalent to `smelt.columns_of(m)` against the underlying `TableExpr`. |

Field access uses dot-notation (`m.path`, `m.name`, `m.tags`, `m.columns`). Field access on any other identifier emits `ModelRefFieldUnknown` at the field span. `ModelRef` is **closed**: the field set is exactly these four fields. Adding a field requires a spec edit and a compiler change.

`ModelRef` is **assignable to `TableExpr`** per `types.md` §"Fragment sort subtyping". The assignability applies wherever a `TableExpr` is required (reducer-`union_all` arguments, `smelt.columns_of` arguments, `FROM`-clause splice positions). The reverse direction (`TableExpr → ModelRef`) does not exist; only values originating from `smelt.models.*` are `ModelRef`-typed.

`ModelRef` is meta-only. It is not a user-writable `smelt.define` parameter or return type, not a list element type users construct in literals, and not a value that reaches the database engine. The internal `SmeltType` witness behind `ModelRef` is unspeced at the user surface; users never construct, deconstruct, or annotate against the witness.

#### `SourceRef` meta record type

`SourceRef` is a closed meta-only record type with the same four-field shape as `ModelRef`:

| Field | Type | Meaning |
|---|---|---|
| `path` | `Text` | Workspace-relative file path of the source's YAML declaration. |
| `name` | `Text` | The source's identifier — the final path segment without the `.yml` suffix. |
| `tags` | `List<Text>` | The source's tag set as declared in the source YAML file. |
| `columns` | `List<ColumnRef>` | The source's column list. Equivalent to `smelt.columns_of(s)` against the underlying `TableExpr`. |

Field access uses dot-notation. Unknown field access emits `SourceRefFieldUnknown` at the field span. `SourceRef` is **closed** and **assignable to `TableExpr`** under the same rules as `ModelRef`. `SourceRef` is meta-only and not user-constructible.

#### Wide-reflection diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|---|---|---|
| `WithTagRequiresText` | `smelt.models.with_tag(x)` or `smelt.sources.with_tag(x)` whose `x` synthesises to a type not assignable to compile-time `Text` | `with_tag expects a compile-time Text; found {actual}` |
| `WithTagNamedArgument` | `with_tag` called with a named argument | `with_tag takes one positional argument; named arguments are not supported` |
| `WideReflectionUnknownAccessor` | `smelt.models.<name>` or `smelt.sources.<name>` where `<name>` is not in the closed accessor set | `smelt.{models,sources} has no accessor `{name}`; expected one of: with_tag, all` |
| `WideReflectionUnexpectedArgument` | `smelt.models.all(x)` or `smelt.sources.all(x)` — any argument to `all` | `{accessor} takes no arguments` |
| `ModelRefFieldUnknown` | Field access on a `ModelRef` value with an identifier outside the closed field set | `ModelRef has no field `{name}`; expected one of: path, name, tags, columns` |
| `SourceRefFieldUnknown` | Field access on a `SourceRef` value with an identifier outside the closed field set | `SourceRef has no field `{name}`; expected one of: path, name, tags, columns` |

#### LSP support for wide reflection

- **Hover** on `smelt.models.with_tag(t)` or `smelt.sources.with_tag(t)` shows `List<ModelRef>` / `List<SourceRef>` and (when `t` resolves to a literal at the cursor) the resolved match count plus the first five matching names.
- **Hover** on `smelt.models.all` / `smelt.sources.all` shows the signature plus the workspace's total count.
- **Hover** on a `ModelRef`-typed binding (a lambda parameter inside a wide-reflection HOF chain) shows `ModelRef` plus the closed field list with each field's type. The corresponding rule holds for `SourceRef`.
- **Hover** on a field projection `m.path` / `m.name` / `m.tags` / `m.columns` shows the field's declared type and, at expansion time over a resolvable list, the field's concrete value at the current element.
- **Goto-definition** on `smelt.models.with_tag` / `smelt.models.all` / `smelt.sources.with_tag` / `smelt.sources.all` resolves to the reference page (URL hint, graceful no-op when the client lacks support).
- **Goto-definition** on a `ModelRef` value at a splice site (where the value has been lifted to `TableExpr` and consumed in a `FROM`-clause or reducer position) resolves to the model's source file. Goto-definition on `m.path` / `m.name` returns the same file. The same rule applies to `SourceRef` resolving to the source YAML file.
- **Completion** at `smelt.models.<cursor>` and `smelt.sources.<cursor>` offers the closed accessor set (`with_tag`, `all`).
- **Completion** at a `ModelRef` / `SourceRef` field projection (`m.<cursor>`) offers the closed field list (`path`, `name`, `tags`, `columns`).
- **Diagnostics with frame stacks**: a type error inside a HOF lambda body whose source list comes from `smelt.models.with_tag(t)` carries the anonymous frame plus an optional `model_origin` field on the per-element entry, recording the source model's `path` and frontmatter declaration span when statically traceable. The corresponding rule applies for `smelt.sources.*`-sourced lists. `model_origin` is the wide-reflection sibling of `column_origin`; the `expansion.md` anonymous-frame contract registers this extension.

### Records

#### `smelt.record` declaration

```
smelt.record TypeName = { field1: Type1, field2: Type2, … }
```

A `smelt.record` declaration introduces a **named record type** at workspace scope. Declarations are top-level (alongside `smelt.define`), one per statement, terminated by a newline. The body is a brace-delimited comma-separated field list; each entry binds a field name to a meta-language type. Trailing commas are permitted. The declared name is workspace-globally unique; a second declaration of the same name emits `SmeltRecordRedefinition` at the offending name token.

Field types may be any meta-language type expressible at type-annotation positions: scalar `DataType` literals (`Text`, `Integer`, `Float`, `Boolean`, `Timestamp`, `Date`, `Decimal`), `List<T>`, `Map<K, V>`, an inline record type `{…}`, or a previously-declared `smelt.record` name. Reflection witnesses (`ColumnRef`, `ModelRef`, `SourceRef`) are **not** writable as field types — `RecordFieldTypeForbidden` at the field's type span.

Declared records are referenced by their bare name in any type-annotation position: as a loader schema (`smelt.config.load_yaml(path, TypeName)`), as a field type of another record, or as a `Map<K, V>` value type. Field-projection on a `TypeName`-typed value uses dot notation (`r.field1`).

#### Inline record types

At any type-annotation position, a brace-delimited field list `{field1: Type1, field2: Type2, …}` is an **inline (anonymous) record type**. Inline records are structurally typed: two inline records with the same field set (names and types, in any order) are the same type. An inline record type is interchangeable with a named record of the same field set per width-subtyping rules below.

Inline record types are the primary surface for one-shot loader schemas: `smelt.config.load_yaml('config.yaml', {threshold: Integer, enabled: Boolean})`. Named declarations (`smelt.record`) are the surface for shapes that recur across files and want their own goto-definition target.

#### Record literals

A record value is constructed by a brace-delimited comma-separated key-value list:

```
{field1: value1, field2: value2, …}
```

The literal is bidirectionally type-checked against its surrounding target type. The target may be a named record (`TypeName`), an inline record (`{f1: T1, …}`), or a HOF/loader return position whose declared type is a record. A record literal in a position where no target type is inferable emits `RecordLiteralUnknownTarget` at the literal's opening brace.

Required-field rules:

- Every field in the target type **must** appear in the literal exactly once. A missing field emits `RecordFieldMissing` at the literal's closing brace, naming the missing field.
- A literal field whose name is not declared on the target type emits `RecordFieldUnknown` at the offending field-name token, listing the closest valid field names.
- A literal that names the same field twice emits `RecordFieldDuplicate` at the second occurrence.
- Each field's `value` must be assignable to the target's declared field type under `types.md` §"Fragment sort subtyping"; mismatches emit `RecordFieldTypeMismatch` at the value expression.

Field order in the literal is **immaterial** to type-checking but **preserved** for diagnostic ordering and for the LSP rename / formatting passes.

#### Field projection

For a record-typed value `r`, the expression `r.fieldname` is **field projection**. It synthesises the declared field's type, lifted into the surrounding splice context per the meta-/Data-world boundary rules. Projection of an unknown field emits `RecordFieldUnknown` at the field-name token, listing valid field names from the closed declared set.

Field projection is recursive: `r.outer.inner` projects `outer`, then `inner`. Each step type-checks independently; a mid-chain projection of a non-record field emits `RecordFieldNotProjectable` at the offending projection.

#### Width subtyping

Record types follow **width subtyping**: a record `{a: T, b: U}` is a subtype of `{a: T}`. A value whose declared type is `{a: T, b: U}` is assignable to any position expecting `{a: T}`. Conversely, a value typed `{a: T}` is **not** assignable to a position expecting `{a: T, b: U}` (the required `b` is missing).

Width subtyping applies uniformly across named declarations and inline records: a value typed `SourceEntry` (declared with fields `{name: Text, columns: List<Text>}`) is assignable to `{name: Text}`; the inverse is not.

Width subtyping does not weaken field-projection diagnostics. A projection `r.b` on a value statically typed `{a: T}` emits `RecordFieldUnknown`; the declared static type, not the runtime / call-site widening, governs the closed set.

#### Record diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|---|---|---|
| `SmeltRecordRedefinition` | A second `smelt.record` declaration in the workspace shares an existing record's name | `record `{name}` is already declared in {path}; record names must be unique workspace-wide` |
| `RecordFieldUnknown` | Field projection or literal field name outside the target's declared field set | `record `{type}` has no field `{name}`; expected one of: {fields}` |
| `RecordFieldMissing` | A record literal omits a field required by the target type | `record literal for `{type}` is missing required field `{name}`` |
| `RecordFieldDuplicate` | A record literal names the same field twice | `field `{name}` already appears in this record literal` |
| `RecordFieldTypeMismatch` | A literal field value's type is not assignable to the declared field type | `record field `{name}` expects {expected}; found {actual}` |
| `RecordLiteralUnknownTarget` | A record literal appears in a position with no inferable target type | `cannot infer record type from context; annotate the target type` |
| `RecordFieldNotProjectable` | Mid-chain field projection through a non-record-typed value | `value of type {type} has no fields; projection `{field}` is not valid` |
| `RecordFieldTypeForbidden` | `smelt.record` field type references a meta-only witness (`ColumnRef`, `ModelRef`, `SourceRef`) | `record field types may not reference {type}; reflection witnesses are not user-writable` |

#### LSP support for records

- **Hover** on a `smelt.record` declaration name (anywhere it appears) shows the full field list with types and the declaration's file path.
- **Hover** on a record-typed binding shows the declared record name (or `{…}` for an inline record), the closed field list with types, and the declaration's path when named.
- **Hover** on a field projection `r.fieldname` shows the field's declared type.
- **Hover** on a record literal's opening brace shows the inferred target type when statically known.
- **Goto-definition** on a `smelt.record` name reference resolves to the declaration site (file + range).
- **Goto-definition** on a record literal's field name resolves to the declared field's source span in the corresponding record type.
- **Completion** at a record literal field-key position (`{<cursor>…}` or `{f1: v1, <cursor>…}`) offers the unfilled field names of the target type, with the field's declared type as the completion-item detail.
- **Completion** at a field-projection site (`r.<cursor>`) offers the record's closed field list with types.
- **Diagnostics with frame stacks**: a `RecordFieldMissing` or `RecordFieldTypeMismatch` inside a HOF body carries the anonymous expansion frame; the frame chains back to the record declaration site (when named) as an informational secondary frame.

### Maps

#### `Map<K, V>` type formation

```
Map<K, V>
```

A `Map<K, V>` value is a meta-only key-value collection. `K` and `V` are meta-language types. In v1 the key type `K` is constrained to be `Text`; a `Map<K, V>` whose declared `K` is not `Text` emits `MapKeyTypeNotText` at the type expression. Future extensions may relax this once an equality-and-hashing surface is specified for additional key types; users must not assume any non-`Text` key type works.

`Map<K, V>` values are produced exclusively by:

- The loader family (`smelt.config.load_yaml`, `smelt.config.load_json`, `smelt.config.load_toml`) when the declared schema is `Map<K, V>` — a YAML mapping at the loaded file's top level (or under a nested record field whose declared type is `Map<…>`) materialises as a `Map`.
- HOF transformations that preserve the map shape (none in v1; future `map_values`-style HOFs may add producers without changing the type).

There is **no literal `Map<…>` syntax** in v1. A user constructing a key-value structure inline writes a `List<Record<{key: K, value: V}>>` — the type the `entries` accessor produces — and consumes it with the existing list HOFs. Adding a `Map` literal requires a spec edit and a new syntactic rule.

`Map<K, V>` is **invariant** in `K` and `V`. A `Map<Text, Integer>` is not a `Map<Text, Number>` even though `Integer <: Number` in the numeric promotion chain. The invariance protects key-lookup semantics: a covariant `K` would admit unsound lookups; a covariant `V` is sound but unmotivated, and matching invariance on both axes keeps the rule simple.

#### Map API

Operations on a `Map<K, V>` value `m` are expressed as method-call syntax:

| Operation | Signature | Result |
|---|---|---|
| `m.entries()` | `Map<K, V> -> List<{key: K, value: V}>` | The key-value pairs as a list, sorted ascending by `key` (lexicographic for `Text` keys). |
| `m.keys()` | `Map<K, V> -> List<K>` | The keys as a list, sorted ascending. |
| `m.values()` | `Map<K, V> -> List<V>` | The values as a list, ordered by their corresponding keys' ascending sort. |
| `m.get(k)` | `(Map<K, V>, K) -> V` | The value bound to `k`. The argument `k`'s type must be assignable to `K`; missing-key behaviour governed by the rules below. |
| `m.has(k)` | `(Map<K, V>, K) -> Boolean` | `TRUE` iff `m` contains a binding for `k`; otherwise `FALSE`. |

The five method names `entries`, `keys`, `values`, `get`, `has` are the **closed Map API**. A method invocation `m.<other>(…)` emits `MapApiUnknown` at the method-name token, listing the closed set. `m.get` and `m.has` require exactly one positional argument; arity mismatches emit `MapApiArityMismatch`. Named arguments are not supported and emit `MapApiNamedArgument` at the named-argument span. `m.entries`, `m.keys`, `m.values` require empty argument lists; a positional or named argument emits `MapApiUnexpectedArgument`.

#### `m.get` missing-key behaviour

`m.get(k)`'s evaluation depends on whether the argument `k` is statically resolvable:

- **Statically-known `k` absent from `m` at meta-evaluation time.** `MapGetMissingKey` at the call expression; the call's evaluated value is `Unknown`, and the surrounding expression's drop-on-error policy (per the existing `MetaSpreadInForbiddenPosition` / `ColumnsOfUnresolvableSchema` model) governs follow-on diagnostics.
- **Statically-known `k` present in `m`.** The call evaluates to the bound value typed `V`; no diagnostic.
- **Non-statically-known `k`.** The call's type is `V`; evaluation deferred to expansion time. At expansion time the same rules apply with the call-site-resolved `k`. A `Map<K, V>` whose contents are themselves resolvable only at expansion time still admits this analysis (the loader emits a value whose contents are fully bound at the load site).

#### Map diagnostic codes

Owned by `crates/smelt-db/src/lib.rs::DiagnosticCode` (all anchored at the offending CST span):

| Code | When | Message shape |
|---|---|---|
| `MapKeyTypeNotText` | A `Map<K, V>` type expression with `K` other than `Text` | `Map key type must be Text in v1; found {type}` |
| `MapApiUnknown` | Method-call on a `Map<K, V>` value with a name outside the closed Map API | `Map has no method `{name}`; expected one of: entries, keys, values, get, has` |
| `MapApiArityMismatch` | `m.get` or `m.has` called with other than one positional argument | `Map.{method} expects one positional argument; found {n}` |
| `MapApiNamedArgument` | A Map API method called with a named argument | `Map.{method} does not support named arguments` |
| `MapApiUnexpectedArgument` | `m.entries`, `m.keys`, or `m.values` called with any argument | `Map.{method} takes no arguments` |
| `MapGetMissingKey` | `m.get(k)` with statically-known `k` absent from `m` | `Map has no binding for key `{key}`` |
| `MapApiArgTypeMismatch` | `m.get(k)` or `m.has(k)` with `k`'s type not assignable to `K` | `Map.{method} expects key of type {expected}; found {actual}` |

#### LSP support for maps

- **Hover** on a `Map<K, V>`-typed binding shows the type, the resolved entry count (when statically known via a loader call site), and the first five keys.
- **Hover** on a method invocation (`m.entries`, `m.keys`, etc.) shows the method's signature and, when the underlying `Map` is statically resolvable, the result's resolved length.
- **Hover** on `m.get(k)` shows the result's declared type `V` and (when `k` is statically known and present) the resolved value's hover.
- **Goto-definition** on the bare method name (`entries`, `keys`, `values`, `get`, `has`) resolves to the reference page (URL hint, graceful no-op when the client lacks support).
- **Completion** at `m.<cursor>` offers the closed Map API method set with arities and signatures as completion-item details.
- **Completion** at `m.get(<cursor>)` and `m.has(<cursor>)` offers the statically-known key list of `m` when resolvable (first ~50 keys), each shown with its bound value's type.
- **Diagnostics with frame stacks**: a `MapGetMissingKey` diagnostic inside a HOF body whose source list comes from `m.entries()` carries the anonymous frame; the frame's `map_origin` field (the load site of `m`) is the wide-reflection-style provenance and registers as a sibling of `column_origin` / `model_origin` on the `expansion.md` anonymous-frame contract.

### Meta-`Text`-as-identifier lift

A meta-`Text` value spliced into a position where the Data-World SQL grammar expects an unquoted identifier lifts to that identifier. The lift positions are exactly:

| Position | Example |
|---|---|
| Column-reference position inside an expression | `COALESCE(c.name, 0)` — `c.name` lifts |
| `AS` alias of a SELECT item | `SUM(amount) AS c.name` — `c.name` lifts |
| `ORDER BY` column reference | `ORDER BY c.name` — `c.name` lifts |
| `GROUP BY` column reference | `GROUP BY c.name` — `c.name` lifts |

In **any other position** — function arguments where the parameter sort is `Expr<Text>`, comparison operands typed `Text`, string-literal positions, named-argument values — a meta-`Text` retains its string-value meaning. The lift is grammar-position-driven, not user-annotated; there is no inverse cast and no opt-in marker.

A lifted identifier is then re-validated against the surrounding splice context's column-resolution scope per `scoping.md`'s standard column-resolution rule. A lifted identifier naming a column not in the surrounding scope emits the existing `UnknownColumn` diagnostic at the lifted identifier's source span (the meta expression's CST node, not the lifted text).

The lift applies **only to compile-time meta-`Text` values**, not to runtime `Expr<Text>` values. A runtime `Expr<Text>` (e.g. `UPPER('foo')`) in an identifier position remains a Data-World type error per existing splice-context rules; the meta lift does not extend to evaluated SQL expressions.

## Semantics

### Two worlds, one program

The meta-language extends smelt with a **compile-time evaluation layer**. Every program is checked and evaluated in two interlocking layers:

- **Meta-World.** Compile-time values (lists, lambdas, records, reflection results, config values). Types are fragment sorts (`Expr<T>`, `TableExpr`, `SelectItems<…>`, `OrderSpec`) plus the new types `List<T>`, `Lambda<…>`, `Record<…>`, `Map<K,V>`. Meta values exist only during compilation.
- **Data-World.** SQL the database engine sees. Types are the `DataType` vocabulary in `types.md`. Data values exist at query runtime.

The two worlds intersect at **splice points** — places where a meta value materialises into Data-World syntax. Splice points already exist (every `smelt.<path>(...)` call is one); the meta-language adds: list literal positions, spread positions, and generated-model positions.

### Meta-evaluation rules (load-bearing)

1. **Termination.** Meta evaluation must terminate without user-visible recursion. Lists are finite; HOFs walk once. Reflection results are bounded by workspace state. The compiler must reject any construct that admits unbounded recursion at meta level.
2. **Determinism.** Meta evaluation given the same workspace state must produce the same result. No clock, no random, no network. Environment variables are accessible only via gated APIs that opt the file out of pure determinism.
3. **Purity.** Meta evaluation has no side effects. The compiler may evaluate the same expression multiple times for caching reasons; user code may not depend on observable side effects.
4. **Single-pass type-check.** Every meta program must type-check in a single pass without execution. The type checker may invoke a (bounded) meta evaluator to compute reflection results during checking, but this evaluator is itself pure and terminates.
5. **No string templating.** Meta values are CST nodes (or values that produce CST nodes). The compiler never re-parses the output of meta evaluation.

### Per-construct semantics

#### Lists and spread

1. **List type formation.** A `List<T>` value is constructed only by a list literal or by a HOF (`map` / `filter`). `T` is resolved at construction time and is invariant for the lifetime of the value.

2. **List literal evaluation.** `[e_1, …, e_n]` evaluates each element `e_i` in the surrounding splice context and produces a `List<T>` value of length `n`. `T` is the LUB of the element types under `types.md` §"Numeric promotion chain". A literal whose elements do not unify is `List<Unknown>` and emits `MetaListHeterogeneous`; downstream consumers of `List<Unknown>` follow the widening rule in `gradual_typing.md` §"List<Unknown> widening".

3. **Bidirectional disambiguation.** The parser produces one `ARRAY_LITERAL` CST node for `[…]`; meaning is assigned by the type checker:
   - If the surrounding target sort is `List<T>` (meta), the literal evaluates as a meta-list with element type `T`.
   - If the surrounding target sort is `Expr<Array<U>>` (Data-World) or any context that admits an array literal per the existing array-literal rules, the literal evaluates as a runtime array.
   - If both are admissible, **meta-list wins**. Users opt explicitly into the runtime-array meaning by writing the `Array<U>(…)` constructor.
   - If neither is admissible, the literal is a type error at the surrounding splice position; the literal itself is `List<Unknown>`.

4. **Empty literal.** `[]` evaluates to an empty `List<T>` if and only if the surrounding context supplies a target sort; otherwise `MetaListEmptyTypeUnknown` and `List<Unknown>`. An empty `Array<U>` literal is permitted only in Data-World positions that already accept zero-length arrays.

5. **Subtyping.** `List<T>` is **covariant** in `T`. If `S <: T` per `types.md` §"Fragment sort subtyping", then `List<S> <: List<T>`. Lists are immutable, so covariance is sound.

6. **Spread evaluation.** `...xs` where `xs: List<T>` materialises into the surrounding comma-separated grammar position by emitting `n` copies of the elements at the spread's source span. Each emitted element retains a `Synthesized(SpreadFrom(span_of(xs)))` provenance origin tag (per `expansion.md` §"Provenance origin tags"). The resulting comma-separated list is then re-validated against the surrounding position's existing kind/type rules.

7. **Spread of empty list.** A spread of any compile-time empty `List<T>` emits zero copies; adjacent commas elide. Re-validation of the surrounding position then runs against the elided form.

8. **Spread position validation.** Spread in any position not enumerated under §"Spread operator" is `MetaSpreadInForbiddenPosition` and is dropped from the surrounding form (so a single misplaced spread does not avalanche follow-on errors).

9. **Spread on non-list.** `...x` where `x` is not a `List<T>` is `MetaSpreadOnNonList`. The spread is dropped; the surrounding position type-checks as if the spread were absent.

10. **Compile-time-only.** No `List<T>` value reaches the database engine. After meta-evaluation, the Data-World CST handed to codegen contains no `ARRAY_LITERAL` and no spread node; every list value has been consumed by spread, by a HOF, by a reducer, or by a record / map / generator.

11. **Termination.** Lists and spread introduce no meta-recursion. List literal evaluation walks the elements left-to-right exactly once; spread walks the source list exactly once. Wall-clock cost is O(n) in the source length.

#### Lambdas and higher-order functions

1. **Lambda formation.** `fn x => body` constructs a `Lambda<T, U>` value where `T` is the HOF-supplied parameter type and `U` is the synthesised type of `body` under the binding `x : T`. A lambda outside a HOF positional-argument position is `LambdaInForbiddenPosition`; a lambda with multi-argument syntax is `LambdaArityNotSupported` (detected via a text shape check on the HOF's second argument). Lambdas are values only — they have no declaration site, no name, no `smelt.<path>` reachability.

2. **Lambda parameter scoping.** Inside `body`, a bare reference to `x` resolves to the lambda parameter before any wider scope (function parameters, CTE columns, `TableExpr`-parameter columns, upstream schemas — see `scoping.md` §"Resolution order"). Lambda parameters are pushed onto the body's `TypeContext` for the duration of the body walk and popped on exit. A lambda parameter shadowing a `smelt.define` parameter or a sibling lambda parameter is permitted (lexical shadowing is the standard meaning); the inner binding wins. The `scoping.md` lambda-scope contract registers lambda parameters as a scope kind.

3. **Lambda capture.** A lambda body may reference: lambda parameters in scope (the immediate parameter and any enclosing lambda parameters); the enclosing `smelt.define`'s parameters; meta-only outer-scope names (`List<T>` values, `smelt.config.var('x')` results). It must not reference SQL columns reachable only at Data-World runtime — those names do not exist at meta-evaluation time. A capture of a runtime-only name surfaces as `UnknownIdentifier` at the bare reference, with the lambda's enclosing HOF call as the diagnostic anchor.

4. **HOF evaluation.**
   - `map(xs, f)`: produces a new `List<U>` of length `len(xs)`, with element `i` equal to the result of applying `f` to `xs[i]`. Evaluation walks `xs` left-to-right exactly once; the lambda body is type-checked once (parametrically), evaluated once per element. Order is preserved.
   - `filter(xs, p)`: produces a sub-list of `xs` (in original order) keeping element `i` if and only if `p(xs[i])` evaluates to `TRUE`. The predicate body must synthesise to `Boolean`; a synthesised `Unknown` propagates per `gradual_typing.md` and the element is dropped (with no error) if the predicate is `Unknown`. Order is preserved.
   - `reduce(xs, r)`: produces a single fragment of the reducer's declared output sort. The reducer is fully resolved at type-check time from the closed registry; the second argument is a bare reducer identifier (not a value-bearing expression).
   - All three HOFs require exactly two positional arguments and zero named arguments. A HOF call with named arguments emits `HofExpectsLambda` or `HofExpectsReducer` at the offending argument expression.

5. **HOF inline expansion frame.** When a diagnostic surfaces from inside a HOF lambda body, the diagnostic carries an **anonymous expansion frame** identifying the HOF and the source-list element index when known. The frame's shape per `expansion.md`'s extended `FrameInfo` contract: `function = "<hof name>"`, `fn_id = None` (no declaration site), `decl_path = None`, `decl_range = None`, `call_site_range = span_of(HOF call)`, plus an optional `element_index` field naming the source list index whose evaluation produced the inner error. The `expansion.md` anonymous-frame contract registers this form; the frame is producible but the LSP renderer currently reads only the call-site range.

6. **HOF and reducer name reservation.** The bare identifiers `map`, `filter`, `reduce`, and every entry in the closed reducer registry (`comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`) are reserved at the meta-namespace level. A `smelt.define` declaration with one of these names emits `HofNameShadowed` or `ReducerNameShadowed` at the declaration's name token. Reserved names cannot be used as `smelt.define` parameters either; a parameter named `map` emits the same diagnostic.

7. **Termination.** HOF constructs terminate structurally:
   - `map` and `filter` walk a finite list once.
   - `reduce` left-folds a finite list once.
   - Lambda bodies are checked once parametrically and evaluated once per HOF iteration; they admit no recursive HOF call against their bound parameter (a lambda body containing a HOF call is permitted, but the inner HOF's source list must come from outer scope — a lambda cannot construct a list whose iteration includes itself).

#### Pipe operator

1. **Pipe desugaring.** `LHS |> CALL(args...)` is equivalent in every observable way to `CALL(LHS, args...)`. This equivalence holds for evaluated value, synthesised type, frame-stack contribution, and diagnostic anchoring. Diagnostics that would have anchored on the un-piped call argument anchor instead on the corresponding source span (LHS expression for the pipe-introduced first argument; original `args` spans for the rest). Pipe never introduces a fresh frame; the desugared call is treated as if the user had written it directly.

2. **Pipe binding.** `|>` is left-associative with the lowest meta-language precedence. `a |> b(p) |> c(q)` parses as `(a |> b(p)) |> c(q)` and desugars to `c(b(a, p), q)`. Pipe never crosses statement boundaries: `; |>` and `|> ;` are parser errors at the offending token.

3. **Pipe RHS.** The RHS must syntactically be a call expression — a function call (`f(args)`), a `smelt.<path>(args)` call, or a HOF (`map(args)`, `filter(args)`, `reduce(args)`). A non-call RHS emits `PipeRhsNotCall`; the pipe expression as a whole evaluates as if `LHS` were the un-piped result and the pipe were absent.

4. **Termination.** Pipe is purely textual desugaring at parse time; it contributes no evaluation step of its own.

#### Contextual reducers

1. **Reducer evaluation.** `reduce(xs, r)` evaluates as follows:
   - If `xs` is non-empty, the result is the reducer's left-fold over `xs` rendered into a single fragment of the reducer's declared output sort (using the reducer's binary-operation rule: `comma_sep` produces `e1, e2, …`; `and_all` produces `e1 AND e2 AND …`; `union_all` produces `e1 UNION ALL e2 UNION ALL …`; etc.).
   - If `xs` is empty and the reducer declares an identity, the result is the identity (e.g. `and_all` → `TRUE`; `concat` → `''`).
   - If `xs` is empty and the reducer has no identity (`union_all`, `intersect_all`), the diagnostic is `ReducerEmptyNoIdentity` and the surrounding splice position emits no fragment (the position re-validates as if the `reduce` call were absent — same drop-on-error policy as `MetaSpreadInForbiddenPosition`).
   - Reducer evaluation is order-preserving (left-to-right). `union_all` and `intersect_all` produce SQL whose row order is the user's reasonable expectation given the source list's order, but smelt makes no row-order guarantee beyond what SQL itself guarantees for the underlying operator.

2. **Reducer input typing.** The reducer's declared input element type must be assignable from the actual list's element type per `types.md` §"Fragment sort subtyping" and §"Numeric promotion chain". Mismatches emit `ReducerInputTypeMismatch`. For `plus_chain` and `concat`, the output element type is the LUB of the inputs (numeric promotion or `Text` widening). For `comma_sep`, the output is `SelectItems<Scalar>` regardless of the inputs' specific `T` (the reducer collapses `List<Expr<T>>` to a select-list shape; per-element type information is preserved at the splice point but not re-exposed as a list type).

3. **Termination.** `reduce` left-folds a finite list once.

#### Compile-time variables

1. **`smelt.config.var` evaluation.** `smelt.config.var('x')` resolves at type-check time:
   - Read the workspace's `smelt.yml` `vars:` block. The path is `<workspace_root>/smelt.yml` (or per-target overlay per `smelt_yml.md`).
   - Look up `'x'` in the `vars:` map. Absent → `ConfigVarNotFound`; non-string scalar → render to `Text` per the rules below.
   - YAML scalar coercion: strings round-trip as their value (`"hello"` → `'hello'`); booleans render as `'true'` / `'false'`; integers and floats render as their decimal representation; `null` renders as `''` and warns `ConfigVarNullCoercion`.
   - The synthesised type is always `Text`. Richer-typed reads require explicit schema declarations.
   - The argument must be a string literal; non-literal arguments emit `ConfigVarNameNotLiteral`.

2. **Termination.** `smelt.config.var` is a single map lookup.

#### Reflection: `smelt.columns_of`, `ColumnRef`, identifier lift

1. **`smelt.columns_of` is a Salsa-cached pure function of workspace state.** The accessor's resolved value is invariant for a given workspace input snapshot. Re-evaluation across two runs over the same workspace produces byte-equal results. The implementation is a Salsa query (per the `smelt-db` Salsa-wrapper rule in `CLAUDE.md`) that reads the upstream schema via the existing `ModelSchema` resolution machinery.

2. **Body-check vs expansion-time evaluation.** `smelt.columns_of(t)` is evaluated in two regimes:
   - **At body-check time** (inside a `smelt.define` body, where `t` is a parameter declared `TableExpr` or `TableExpr<{…}>`): the result type synthesises to `List<ColumnRef>` parametrically. The lambda body of any HOF over the result checks against `ColumnRef` per element. No concrete column list is materialised at body-check time.
   - **At expansion time** (when the function is inlined at a call site with a concrete `t`): the call-site schema for `t` is resolved via the standard `smelt.<path>` resolution (`architecture.md` §"Resolution"), the list of `ColumnRef` values is materialised, HOF lambdas are walked per element, and any meta-`Text`-as-identifier lifts are validated against the surrounding splice context's column-resolution scope.

3. **Source-schema resolution.** `smelt.columns_of(t)` resolves `t`'s schema through the same path as Data-World column-reference resolution:
   - `smelt.<path>` to a model / source / seed → the `ModelSchema` for that path.
   - A `smelt.define` `TableExpr` / `TableExpr<{…}>` parameter at expansion time → the call-site argument's resolved schema (per `expansion.md`'s body-walk-with-bound-parameters rule).
   - A CTE alias inside the same body → the CTE's synthesised schema.
   - Any other `TableExpr`-typed expression (a subquery, a join expression) → the standard schema-resolution path; if no schema can be derived (e.g. upstream `Unknown`), the expansion emits `ColumnsOfUnresolvableSchema`.
   - A `TableExpr<{required columns}>` parameter contributes only the *required* columns to body-check-time `columns_of` reasoning. At expansion time the call-site schema (which may include extra columns under the row-tail per `types.md` §"`TableExpr` row polymorphism") supplies the full list.

4. **`ColumnRef` field projection.** Inside any context where a `ColumnRef`-typed value is in scope (a lambda parameter bound by a HOF over `List<ColumnRef>`), the dot-notation `c.<field>` synthesises the declared field's type:
   - `c.name : Text` (a meta-`Text` value)
   - `c.type : DataType` (a meta literal — comparable for equality, usable in checks like `c.type == Integer`; not user-writable in Data-World annotations per `types.md`)
   - `c.is_numeric : Boolean`
   Any other field name emits `ColumnRefFieldUnknown` at the field span.

5. **`ColumnRef` ordering.** The `List<ColumnRef>` produced by `smelt.columns_of(t)` preserves the source schema's declared column order. For models, sources, and seeds this is the order columns appear in their schema declaration. For function `TableExpr` parameters at expansion time this is the order columns appear in the call-site argument's schema.

6. **Meta-`Text`-as-identifier lift.** A meta-`Text` value spliced into one of the four lift positions (column-reference, AS-alias, ORDER BY column-reference, GROUP BY column-reference) is rendered as that identifier in the produced SQL. The lifted identifier is then validated against the surrounding splice context's column-resolution scope per `scoping.md`'s standard column-resolution rule (`UnknownColumn` if the lifted identifier names no in-scope column). The lift produces no expansion-time diagnostic of its own; it is invisible to the type system except as the identity transform `Text → identifier`.

7. **Lift narrowness.** The lift applies only to meta-`Text` values, not to runtime `Expr<Text>` values. A runtime `Expr<Text>` in an identifier position remains a Data-World type error per existing splice-context rules. The lift applies only in the four enumerated positions; in any other position a meta-`Text` retains its `Text` value (e.g. as the operand of `||`, as a function argument typed `Expr<Text>`, as a comparison RHS).

8. **`ColumnRef` is not user-constructible.** `ColumnRef` values originate only from `smelt.columns_of` and from later reflection accessors. The internal `SmeltType` witness behind `ColumnRef` is unspeced at the user surface; user code may not construct, deconstruct, or annotate against the witness. The user-writable record surface is a separate construct that does not retroactively expose `ColumnRef`'s structure.

9. **Reflection determinism.** Reflection results are deterministic functions of workspace state (per the load-bearing meta-evaluation rule in §"Two worlds, one program"). `smelt.columns_of` performs no I/O, makes no network call, observes no clock or random source. The Salsa query layer guarantees re-evaluation produces identical results until the workspace input changes; LSP responsiveness is preserved by automatic invalidation when an upstream schema changes.

10. **Termination.** `smelt.columns_of` performs a single bounded lookup of the source schema's column list. Field projection is a single named-field lookup. Identifier lift is a single tag transformation at expansion time. All three are O(1) per invocation; the surrounding HOF walks the resulting `List<ColumnRef>` once per the HOF termination rule.

11. **HOF inline-expansion frame, `column_origin` extension.** The anonymous expansion frame (`function = "<hof>"`, `fn_id = None`, optional `element_index`) is extended for `columns_of`-sourced lists with an additional optional field `column_origin`: the source span of the column's declaration in the upstream `ModelSchema`. When a diagnostic surfaces from inside a HOF lambda body whose source list came from `smelt.columns_of(t)`, the frame's `column_origin` carries the source column's span when statically resolvable. The `expansion.md` anonymous-frame contract registers this extension; producers populate the field, the LSP renderer surfaces it as a "from column declared at <span>" trailer when present.

#### Reflection: `smelt.models`, `smelt.sources`, `ModelRef`, `SourceRef`

1. **Wide-reflection accessors are Salsa-cached pure functions of workspace state.** `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, and `smelt.sources.all` are Salsa queries reading the `Workspace` singleton input (per `crates/smelt-db/src/lib.rs`'s existing `all_models` and `project_sources` queries). Re-evaluation across two runs over the same workspace input produces byte-equal results — list contents, ordering, and per-element field values are deterministic.

2. **Ordering.** `with_tag` and `all` return lists sorted ascending by `path`. Path comparison is byte-lexicographic on the workspace-relative path string with `/` separators. The order is observable by users; row order in a `reduce(union_all)` over a wide-reflection result follows this order.

3. **`with_tag` tag matching.** A model matches `smelt.models.with_tag(tag)` iff its merged tag set (frontmatter `tags:` ∪ `smelt.yml` `models.<name>.tags`, deduplicated by `Config::get_tags`) contains `tag` by exact string equality. A source matches `smelt.sources.with_tag(tag)` iff the source YAML's `tags:` list contains `tag` by exact string equality. No case-folding, no glob matching, no prefix matching.

4. **Body-check vs expansion-time evaluation.** Wide-reflection accessors are evaluated under the same two-tier regime as `smelt.columns_of`:
   - **At body-check time** (inside a `smelt.define` body): the result type synthesises to `List<ModelRef>` / `List<SourceRef>` parametrically. HOF lambda bodies over the result check against `ModelRef` / `SourceRef` per element. No concrete model or source list is materialised at body-check time.
   - **At expansion time** (when the function is inlined at a call site): the workspace state is read, the matching list is materialised, HOF lambdas are walked per element, field projections resolve to concrete values, and any `ModelRef` / `SourceRef` consumed at a splice point lifts to its underlying `TableExpr`.

5. **`ModelRef` / `SourceRef` as `TableExpr`.** A `ModelRef` value's `TableExpr` projection is the same `TableExpr` that `smelt.<path>` resolves to for that model (`architecture.md` §"Resolution"). A `SourceRef` value's projection is the same `TableExpr` that the source's `smelt.<source_path>` resolves to. The subtyping rule (`ModelRef <: TableExpr`, `SourceRef <: TableExpr`) lifts the value wherever `TableExpr` is required; `smelt.columns_of(m)` and `m.columns` are equivalent, and `reduce([m1, m2], union_all)` and `reduce([m1.<TableExpr-projection>, m2.<TableExpr-projection>], union_all)` produce identical SQL.

6. **`ModelRef.columns` / `SourceRef.columns` materialisation.** `m.columns` is equivalent to `smelt.columns_of(m)` over the underlying `TableExpr`. Body-check time produces parametric `List<ColumnRef>`; expansion time materialises the concrete list per the rules in §"Reflection: `smelt.columns_of`, `ColumnRef`, identifier lift".

7. **Field projection.** Inside any context where a `ModelRef`-typed value is in scope (a lambda parameter bound by a HOF over `List<ModelRef>`), the dot-notation `m.<field>` synthesises the declared field's type. Inside any context where a `SourceRef`-typed value is in scope, the same applies. Any field name outside the closed four-field set emits `ModelRefFieldUnknown` / `SourceRefFieldUnknown` at the field span.

8. **Identifier lift carries through `ModelRef.name` and `ModelRef.path` only at the four enumerated lift positions.** `m.name` and `m.path` are meta-`Text` values; in one of the four lift positions (column-reference, AS-alias, ORDER BY column-reference, GROUP BY column-reference) they lift to identifiers per §"Reflection: `smelt.columns_of`, `ColumnRef`, identifier lift" rule 6. The `FROM`-clause splice that consumes a `ModelRef` as a `TableExpr` goes through the `ModelRef <: TableExpr` subtyping rule, **not** through the identifier-lift path; the lift positions table is not extended by wide reflection.

9. **Wide-reflection-sourced HOF frames, `model_origin` extension.** The anonymous expansion frame (`function = "<hof>"`, `fn_id = None`, optional `element_index`) is extended for wide-reflection-sourced lists with an additional optional field `model_origin` (or `source_origin` for `smelt.sources.*`-sourced lists): the source `path` and the frontmatter declaration span when statically traceable. When a diagnostic surfaces from inside a HOF lambda body whose source list came from `smelt.models.*` / `smelt.sources.*`, the frame carries this provenance. The `expansion.md` anonymous-frame contract registers this extension as the wide-reflection sibling of `column_origin`.

10. **Determinism.** Wide-reflection results are deterministic functions of workspace state (per the load-bearing meta-evaluation rule in §"Two worlds, one program"). The accessors perform no I/O, observe no clock or random source, and make no network calls. The Salsa query layer guarantees re-evaluation produces identical results until a workspace input changes; an edit that adds or removes a frontmatter tag invalidates only the queries whose result depends on that tag's membership.

11. **Closed accessor set.** `smelt.models` exposes exactly `{with_tag, all}`. `smelt.sources` exposes exactly `{with_tag, all}`. Future accessors require a spec edit and a compiler change. The diagnostic `WideReflectionUnknownAccessor` is the user-facing surface of the closed set; misuse anchors at the unknown accessor name token.

12. **Termination.** Each wide-reflection accessor performs one bounded scan of workspace state. In the worst case the scan is O(workspace-size); Salsa memoisation makes repeated evaluations O(1) until invalidation. HOF walks over the result list are governed by the existing HOF termination rule.

#### Records

1. **`smelt.record` declaration evaluation.** A `smelt.record TypeName = { fields }` statement registers a workspace-scoped record-type declaration. Declarations are collected by the workspace-level parser before any model's body is type-checked; this ordering guarantees that any model referencing a declared record name resolves the reference at body-check time regardless of which file the declaration lives in. A second declaration of the same name (across any pair of files in the workspace) emits `SmeltRecordRedefinition` at the second declaration's name token, retaining the first declaration as authoritative for downstream uses (so a single duplicate does not avalanche errors at every consumer).

2. **Field-type validation at declaration time.** Each field's declared type expression is type-checked against the closed set of admissible meta-language types: scalar `DataType` literals, `List<T>`, `Map<K, V>` (with `K = Text`), inline records `{…}`, and previously-declared `smelt.record` names. Reflection witnesses (`ColumnRef`, `ModelRef`, `SourceRef`) and meta-only types whose surface is not user-writable (`Lambda<…>`) emit `RecordFieldTypeForbidden` at the offending field's type expression. The closed admissibility set keeps records from leaking internal type witnesses into user-writable surface.

3. **Cyclic declarations.** A record declaration referencing its own name (directly or transitively through other record declarations) emits `RecordCyclicDeclaration` at the cycle's introducing field-type expression. Mutually recursive records are deferred to a future spec (see §Out-of-scope); v1 record declarations form a DAG.

4. **Inline record types are structurally typed.** Two inline record type expressions with identical field name-and-type sets (in any order) are the same type. A value typed against an inline record is assignable to a named record with the same field set, and vice versa. The named-record name carries metadata (declaration site, hover signature) but does not introduce nominal distinction at the type-checking level.

5. **Record literal evaluation.** A `{f1: v1, f2: v2, …}` literal evaluates each value expression in the surrounding splice context and constructs a record value bound to the target's declared type. Evaluation order is the literal's left-to-right field order; type-checking is independent per field (one field's error does not poison later fields). The resulting value's type is the target type (named or inline). A record literal with no inferable target emits `RecordLiteralUnknownTarget` at the literal's opening brace; the literal evaluates as `Record<Unknown>` for the surrounding expression's drop-on-error policy.

6. **Required-field enforcement.** Every field declared on the target record type must appear exactly once in the literal. A missing field emits `RecordFieldMissing` at the literal's closing brace, naming the missing field. A duplicate field emits `RecordFieldDuplicate` at the second occurrence's name token. A literal field naming a key not declared on the target emits `RecordFieldUnknown` at the offending name token; the value expression is dropped from the constructed record (so a single misnamed field does not avalanche follow-on errors). Field-name matching is byte-equal (case-sensitive).

7. **Field-projection evaluation.** `r.fieldname` where `r` has record type `R` synthesises the declared type of `R`'s `fieldname`. If `R` does not declare `fieldname`, `RecordFieldUnknown` is emitted at the field token; the projection evaluates to `Unknown` for the surrounding expression's drop-on-error policy. Field projection through a non-record-typed value emits `RecordFieldNotProjectable` at the projection token.

8. **Width subtyping.** A record type `R₁` is a subtype of `R₂` if and only if every field declared in `R₂` is declared in `R₁` with an assignable type (per `types.md` §"Fragment sort subtyping"). Width subtyping is one-directional: extra fields in the subtype are silently absorbed at the assignment site (the wider value's extra fields are retained at the value level but invisible to the static type checker's projection rules at the narrower binding). This matches `List<T>`'s covariance treatment — both rely on values being immutable, so unsoundness from mutation is structurally impossible.

9. **No record-shape inference at the literal.** A `{a: 1, b: "x"}` literal in a position with no target type does not synthesise to an inferred inline record type `{a: Integer, b: Text}`; instead it emits `RecordLiteralUnknownTarget`. Annotating the target type (a `smelt.define` parameter, a loader schema, a HOF return shape) is required. The non-inference rule keeps records from acquiring accidental shapes through partial editing — a user who renames a target's field expects the literal to fail loudly at the literal, not silently retype the surrounding code.

10. **Records are pure meta-world values.** A record value never reaches the database engine. Records consumed at splice points lift to the surrounding context (record fields holding `TableExpr`-typed values project per the field, the projection then enters the splice). The record value itself has no Data-World representation; mention of a record-typed binding in a non-splice Data-World position emits `RecordInDataWorld` at the binding reference.

11. **Termination.** Record literal evaluation walks the field list left-to-right exactly once. Field projection is a single named-field lookup. Width subtyping is a single set inclusion check at assignment time. All three are O(n) in the number of fields, with n bounded by the declaration.

#### Maps

1. **`Map<K, V>` type formation.** A `Map<K, V>` type expression registers a meta-only collection type with key type `K` (constrained to `Text` in v1) and value type `V` (any meta-language type). A `Map` type expression with `K` other than `Text` emits `MapKeyTypeNotText` at the type expression; the surrounding declaration treats the map as `Map<Text, V>` for the rest of the body to avoid avalanche errors.

2. **Map value materialisation.** A `Map<K, V>` value originates only from the loader family. When a loader's declared schema is `Map<K, V>` (or contains a `Map<K, V>` field), the parsed file's mapping is materialised as a `Map` value with keys ordered ascending per the rule below. The materialisation is deterministic (same file → same `Map` value, byte-equal).

3. **Iteration order.** `m.entries()`, `m.keys()`, and `m.values()` return lists ordered ascending by key. Key comparison is byte-lexicographic on the `Text` representation. Order is observable by users; HOF walks over `m.entries()` see entries in this sorted order. This matches `smelt.models.with_tag`'s path-sorted ordering rule (predictable, edit-stable, no dependency on file-format-internal order).

4. **Map API evaluation.**
   - `m.entries()` materialises a `List<{key: K, value: V}>` of length `len(m)`, each element a record literal with field `key` bound to the entry's key and `value` to the entry's value. The element record's type is the inline structural record `{key: K, value: V}`. Iteration is left-to-right in sorted order.
   - `m.keys()` materialises a `List<K>` whose elements are the entry keys in sorted order.
   - `m.values()` materialises a `List<V>` whose elements are the entry values in the order their keys sort.
   - `m.get(k)` is a single keyed lookup. Statically-known `k` present → value typed `V`; statically-known `k` absent → `MapGetMissingKey` at the call expression, evaluation `Unknown`; non-statically-known `k` → call type `V`, evaluation deferred to expansion time. The argument's type must be assignable to `K`; mismatch emits `MapApiArgTypeMismatch`.
   - `m.has(k)` is a single keyed presence check; the result type is always `Boolean`. The argument's type rule matches `m.get`. Static-key resolution returns the boolean directly; non-static keys defer to expansion time.

5. **Closed Map API.** The five method names `entries`, `keys`, `values`, `get`, `has` are the entire Map surface in v1. A method call `m.<other>(…)` emits `MapApiUnknown`. The closed-set diagnostic mirrors `ColumnRefFieldUnknown` and `ModelRefFieldUnknown`. Future Map methods (`map_values`, `merge`, `filter_keys`) require a spec edit and a compiler change.

6. **Invariance.** `Map<K, V>` is invariant in both `K` and `V`. A `Map<Text, Integer>` is not assignable to a position expecting `Map<Text, Number>`, even though `Integer <: Number`. The invariance is a deliberate conservatism: covariant `K` admits unsound lookups (a `Map<NarrowKey, V>` typed as `Map<WiderKey, V>` accepts wider-key lookups that have no binding), and covariant `V` is sound but inconsistent with `K`'s invariance — uniform invariance is simpler to teach and matches `Lambda<S, T>`'s rule. Width subtyping over record-typed `V` is handled at the projection of `m.entries()[i].value`, not at the `Map` type level.

7. **Maps are pure meta-world values.** A `Map<K, V>` value never reaches the database engine. Map consumers transform a `Map` into a `List` (via `.entries()` / `.keys()` / `.values()`) and consume the resulting list with existing HOFs; the `Map` itself is materialised, traversed, and discarded at meta-evaluation time.

8. **Termination.** Map API method calls are O(log n) for `m.get(k)` / `m.has(k)` (one bounded-depth lookup), O(n) for `m.entries()` / `m.keys()` / `m.values()` (a single linear materialisation). `n` is bounded by the loader-supplied file's size; the file is itself bounded by workspace state.

#### Pipe ↔ HOF interaction

`smelt.config.var` and HOFs compose with pipe: `smelt.columns_of(orders) |> filter(fn c => c.is_numeric) |> map(fn c => COALESCE(c.name, 0))` is the standard worked-example shape. Because pipe is parser-level desugaring (§"Pipe operator"), the result has the same evaluated type, value, frame-stack contribution, and diagnostic anchoring as the equivalent un-piped call.

## Design

### Why a meta-language at all

`smelt.define` (`functions.md`) closed the gap on **fragment-level** reuse — predicates, expressions, table transformers, select-list shapes can be parameterised, called by name, and inlined with full type checking. It does not address the class of dbt patterns where the *input to the SQL is computed from the project itself*: union all models matching a tag, coalesce all numeric columns, generate one staging model per source-table entry in a YAML file. These patterns require iterating over compile-time data — a list of models, a list of columns, a list of config rows — and reducing the result into a SQL fragment that splices into a model.

dbt does this through Jinja string-substitution. That choice forfeits typing, navigation, and LSP feedback inside macros: `{{ col.name }}` resolves to nothing, errors anchor to the post-substitution SQL, refactoring is text-only. smelt's meta-language proposes to give every meta value a type, every cross-reference an LSP-resolvable definition, every diagnostic a source span. The unifying claim, restated from the research doc: smelt already has a meta-world (fragment sorts, two-tiered expansion, splice contexts); making it user-visible is a layering exercise, not a new language inside the language.

The full design rationale, alternatives considered at every level (lambda surface, reducer registry, list literal disambiguation, reflection API shape, multi-model production mechanism), and the framing of the meta-/data-world boundary live in `docs/research/20260507-typed-meta-programming.md`. This spec records the decisions; the research doc records why they look like this.

### Why a single spec rather than per-construct specs

The constructs are deeply interdependent. Lambdas have no use without HOFs; HOFs have no use without lists; lists have limited use without reflection; reflection has limited use without records; records have limited use without loaders. Splitting this into seven specs would force every later spec to repeat the framing of the meta-/data-world boundary and the meta-evaluation rules above. One spec, one framing, multiple Surface entries grouped by feature.

The exception is `meta_config_loading.md` — the file-loading family is large enough (formats, schema authoring, validation diagnostics, per-target overlay) to warrant its own spec, with this one referencing it.

### Lists and spread — design rationale

**Why one parameterised `List<T>` rather than per-use list types.** The closest existing type is `SmeltType::SelectItems { kind, context }`, but it is contextually constrained (carries an `ExprKind` ceiling and a context binding) and only appears at SELECT-list splice points. A user writing `union by tag` wants a `List<TableExpr>`; a user writing `coalesce(*numerics)` wants a `List<Expr<Numeric>>`; a generator wants a `List<ModelDef>`. Research §4.1 alternative (ii) — `ExprList<T>`, `TableList`, `OrderList` — was rejected because it forces a new type per use case, none of which compose. `List<T>` is the smallest type-theoretic addition that handles every demand. The existing `SelectItems<…>` is preserved (the meta-language does not retire it); the two coexist, and the spec specifies exactly when `List<Expr<T>>` may be used where `SelectItems<Scalar>` is expected.

**Why bidirectional disambiguation rather than a distinct sigil.** Research §4.2 alternative (ii) — `${...}` / `meta[...]` / `(| … |)` — was rejected because users would have two surface forms for similar things. Bidirectional checking is already pervasive in smelt (numeric promotion, `Concrete(T)` resolution, row-variable binding); adding one more bidirectional rule is in-character. The cost — non-local meaning during partial editing — is real; the LSP mitigates it by showing "literal accepted in two contexts; current context expects `List<T>` / `Array<U>`" on hover. Function-style constructors (`list(a, b, c)`) were rejected because the bare name `list` is a likely user identifier and the variadic constructor reads worse than `[…]` for long lists.

**Why "meta-list wins" when both readings are valid.** The only Data-World position that genuinely admits both meanings today is the `Expr<Array<U>>` slot. Defaulting to meta keeps users (who are here to write meta code; the alternative does not yet exist) on the path that motivates the work. Once `Array<U>(…)` ships, users who want the runtime array opt in explicitly; the implicit lift remains meta-first. The reverse default (Data wins) would force every meta user to type-annotate the call site to suppress the array reading, which is exactly the kind of ceremony §"Why a meta-language at all" rejects.

**Why `...xs` rather than always-explicit reducers.** Research §4.3 alternative (iii) — every reduction is a `comma_sep(xs)` / `union_all(xs)` call — was rejected because the common case (splat into a comma-separated grammar position) reads worst when stripped of the spread sugar. Spread keeps the common case terse; reducers remain available for boolean composition, table-set composition, and expression-tree composition where there is no default reduction. `*xs` (Python style) was rejected because `*` is heavily SQL-loaded (`SELECT *`, multiplication); `...` is currently unused in smelt's grammar and a one-token lookahead distinguishes it from any malformed identifier.

**Why covariant subtyping.** `List<T>` is immutable in this language; the standard objection to covariance — a write through the supertype writes a wrong-typed value — does not apply. Mainstream typed languages (Java's wildcards, Scala's `List`, Kotlin's `List`) all expose covariant immutable lists. Variance policed at the spec level keeps the LUB rules sound and matches what users expect from immutable containers. Invariance was considered and rejected because it forces every use site to call `map(_, identity)` to widen, which is friction with no payback.

**Why the empty-list rules are bidirectional and not "always inferable".** A `[]` whose target type cannot be inferred from context is a type error, not a `List<Bottom>` / `List<Nothing>` placeholder. Adding `Bottom` to the meta-type vocabulary is a load-bearing decision that benefits no example; introducing it would propagate into every later LUB rule without any user-visible payback. The diagnostic `MetaListEmptyTypeUnknown` is the simpler answer: tell the user, suggest a target-typed annotation, move on.

### Lambdas and HOFs — design rationale

**Why `fn x => body` rather than position-based `x => body`.** Research §4.5 lists seven candidate lambda surfaces; the contender pair was (i) position-based bare `=>` and (iv) keyword-prefixed `fn`. Position-based disambiguation has the same surface as named arguments (`name => value`); the type checker has to decide which one the user meant by looking at whether the surrounding argument is positional or named. That decision is expensive to surface in error messages ("you wrote `c => CAST(c)` in a positional argument; we read it as a lambda; did you mean a named argument named `c`?") and brittle under partial editing — flipping the surrounding argument from positional to named silently retypes the construct from lambda to named-arg. The `fn` keyword is one identifier of ceremony that locks the meaning at the token boundary; a parser sees `fn`, knows the next identifier is a lambda parameter, and any error message about a misplaced lambda anchors on `fn` itself. Multi-arg lambdas (`fn (a, b) => body`) reuse the keyword cleanly. Backups (i), (ii) `\x . body`, (iii) `|x| body`, (v) implicit `_`, (vi) parens-required `(x => body)`, and (vii) SQL-comprehension `for c in cols select …` are all rejected for reasons in research §4.5; the keyword path costs the least in user-facing error message clarity.

**Why HOFs accept exactly two positional arguments and no named arguments.** The proposed signatures are stable: `map(xs, f)`, `filter(xs, p)`, `reduce(xs, r)`. Allowing named arguments (`map(xs => cols, f => fn c => …)`) collides with the lambda surface (the second arg is a lambda value, not a named-arg form). Allowing additional positional arguments would require committing to a HOF variadic surface ahead of evidence (`zip_with` is the closest contender, and it's a separate built-in name, not a `map` overload). Two arguments, positional only, is the smallest surface that compiles and is the surface every HOF user expects from JS, Python, OCaml, F#, Elixir, etc.

**Why lambda formation is restricted to HOF positional arguments.** A lambda value escaping into a non-HOF position (a list element, a `smelt.define` argument, a record field) would force the meta-type system to treat `Lambda<T, U>` as a first-class member of every type's substructure — propagating the type into list inference, record fields, generic substitution, etc. None of that pays off because the only consumers of `Lambda<T, U>` are the three HOFs. Restricting lambda construction to HOF positional argument positions keeps `Lambda<T, U>` invisible to the surrounding type system; the diagnostic `LambdaInForbiddenPosition` makes the restriction concrete. When a future surface needs first-class lambdas (e.g. user-defined HOFs in a post-plan extension), the restriction is the load-bearing thing to relax.

**Why HOF and reducer names are reserved rather than overloadable.** Allowing a `smelt.define` named `map` would force the type checker to disambiguate at every call site between the built-in HOF and the user's function. The disambiguation rule would be either "user wins" (which silently retires the built-in for that workspace) or "built-in wins for two-arg calls with a lambda" (which couples disambiguation to argument shape, propagating into error messages). Reserving the names produces an immediate, anchored diagnostic at the conflicting `smelt.define` declaration. The cost is that workspaces with pre-existing functions named `map` must rename them; the benefit is that every meta-language user reads `map(xs, f)` as the same operation, regardless of workspace. The same argument applies to reducer names.

### Pipe — design rationale

**Why pipe is first-arg, meta-only, and purely sugar.** Research §4.6 argues for first-arg over last-arg because HOFs naturally take their data first; matching Google Pipe SQL and DuckDB Pipe is the right ecosystem signal. Last-arg (F# style) would force every HOF signature to flip; placeholder pipe (`|>` with `_`) loses the terseness of the common case. Meta-only scope (alt a) keeps the surface focused — pipe-SQL extension (alt b) is a separate paper that extends the SQL grammar and the planner, not the meta-language. Purely-sugar semantics means the type checker can desugar before checking: a pipe expression and the equivalent un-piped call have identical synthesised types, evaluation results, frame-stack contributions, and diagnostic anchoring, modulo the pipe-introduced LHS span. There is no "pipe value" type and no pipe-aware codegen; once the parser is past `|>`, the rest of the pipeline is unchanged.

### Reducers — design rationale

**Why a closed reducer registry (research §4.7 alt (i)).** A user-defined reducer would need to assert associativity (so the compiler can fold in any order), an identity element, and the type system tracking those properties — alternatives (ii), (iii), and (iv) in research §4.7 each require either trust-without-verification or a bigger language change (type classes / monoid instances) than the meta-plan budgets. A closed registry of seven reducers is enough for every dbt-style use case (comma-separated SELECTs, AND/OR composition, table unions, numeric sums, text concatenation). The cost is that adding a reducer requires a compiler change; the benefit is that every reducer's empty-list identity is vetted and every user gets predictable semantics. Parameterised reducers (`concat_with(sep)`) extend the registry's expressive power without opening it to user definition — that is the right axis to grow when concrete pain emerges.

### Compile-time variables — design rationale

**Why `smelt.config.var` is the only non-reflection workspace-state surface.** The user-facing motivation for compile-time variables — per-environment dimension lists, threshold values, feature flags — pre-exists reflection of any kind: it is a project-level configuration concern, not a workspace-introspection concern. Shipping `smelt.config.var` lets users write env-conditional examples without waiting for `smelt.columns_of` or `smelt.models.with_tag`. The literal-only argument restriction is a deliberate scope cut: expression-valued lookups (`smelt.config.var(other_var)`) require resolving an arbitrary `Text`-valued expression at type-check time, which interacts with the loader family's determinism rules. Holding that off keeps the variable-lookup story self-contained.

### Reflection and identifier lift — design rationale

**Why narrow reflection (`smelt.columns_of` / `ColumnRef`) before wide reflection.** The narrow accessor exposes a smaller integration surface — one accessor, one record type, one identifier-lift family — and the wide accessor's `ModelRef` inherits the integration patterns the narrow accessor commits to. Sequencing wide reflection first was rejected for the same reason: every later reflection accessor and the multi-model production mechanism plug into the same expansion-time evaluation regime that the narrow accessor establishes.

**Why `ColumnRef` is a closed record.** Research §4.8 sketched `ColumnRef` as a meta-only type with `name`, `type`, `is_numeric` accessors. Two alternatives were considered: (i) expose `ColumnRef` as a `Record<{…}>` instance (anticipating the record surface) and let users do generic record operations on it; (ii) keep `ColumnRef` as an opaque type with a closed accessor set. Option (i) entangles reflection with the user-writable record surface (which has its own design uncertainties) and leaks the per-field representation into a stable user surface; option (ii) is the closed-registry pattern that worked for reducers. The closed surface keeps reflection self-contained and matches the `LambdaInForbiddenPosition` discipline — an internal type with a narrowly-typed surface, expanded only when concrete demand arises.

**Why `c.is_numeric` is a derived field rather than `c.type.is_numeric`.** The latter requires elevating `DataType` to a meta value with method-call surface (`Integer.is_numeric`), which is a substantial type-system change with no reflection-stage use case. The former is a single accessor whose semantics are pinned by `types.md` §"Type constraints" — the spec rule "is_numeric iff `type` ∈ Numeric constraint set" tells the user exactly what they get. Adding `is_ordered`, `is_temporal`, etc. follows the same pattern when concrete demand arises; the closed-registry contract makes the addition explicit.

**Why the meta-`Text`-as-identifier lift is narrow.** Research §8 noted that `c.name` in `COALESCE(c.name, 0) AS c.name` plays both as a column reference and an identifier alias, and that the crossing rule needs adversarial testing. The narrow rule — lift in exactly four enumerated grammar positions (column-reference, AS-alias, ORDER BY, GROUP BY) — is the smallest commitment that handles the `coalesce_numeric` worked example without committing to a general `Text → identifier` cast. Wider rules considered: (i) lift everywhere a syntactic identifier could appear (CTE names, table aliases, function names) — rejected because it gives users an implicit lift in positions they don't expect, and the resulting Jinja-style "string-becomes-anything" surface defeats the type-system guarantees; (ii) require an explicit `as_identifier(c.name)` cast — rejected because it adds friction at the most common use case (per-column SELECTs in HOF bodies) without payback. The narrow rule is the dial that can widen later under concrete pressure; widening is a spec edit with named additions to the lift positions table.

**Why the lift operates on meta-`Text` only and not `Expr<Text>`.** A runtime `Expr<Text>` (`UPPER('foo')`) cannot be evaluated at compile time, so its "string value" is unknown at the splice point. Lifting it would require executing the expression at compile time (forbidden by §"Meta-evaluation rules" determinism) or generating SQL that uses the expression as an identifier (which is not standard SQL — an identifier must be a parse-time identifier, not a runtime value). The lift is a strictly compile-time operation on compile-time text. Users wanting runtime identifier construction must use a different mechanism; the language deliberately does not provide one.

**Why `smelt.columns_of` accepts only `TableExpr` (not strings, paths, or model names).** Allowing string-typed arguments (`smelt.columns_of('orders')`) would require the type checker to resolve a `Text` value to a model at type-check time, which couples the meta-language to the path-resolution machinery in a non-`TableExpr` axis. Accepting `smelt.<path>` resolves through the existing pipe (every `smelt.<path>` evaluates to a `TableExpr`); accepting `smelt.define` parameters captures the function-body case. The single-axis surface (one parameter type) pins the resolution path through the existing Data-World schema-resolution machinery without minting a parallel one.

**Why expansion-time evaluation rather than body-check-time.** A `smelt.define` body is type-checked once parametrically; resolving `smelt.columns_of(t)` at body-check time would require type-checking the body once *per call site*, which is the opposite of how `expansion.md` partitions checking from inlining. Expansion-time evaluation matches the existing two-tier model: the body checks against `List<ColumnRef>` parametrically, and the inliner walks the per-call-site list to produce concrete SQL. This also matches research §6.4's promise: "the output schema of a model using `coalesce_numeric` is therefore *known at compile time*" — the inliner statically computes the per-call schema, and that schema is what flows downstream to model schemas.

### Wide reflection — design rationale

**Why `smelt.models` and `smelt.sources` rather than `smelt.workspace.models`.** The namespace alternatives considered were (i) `smelt.workspace.models` / `smelt.workspace.sources` (an explicit `workspace.` prefix); (ii) `smelt.models` / `smelt.sources` (a top-level namespace per entity kind); (iii) a unified `smelt.workspace.entities` returning a heterogeneous list. Option (iii) requires sum types over `ModelRef` / `SourceRef`, which are out of scope per §"Out-of-scope by deliberate choice". Option (i) reads cleanly but adds a namespace level that pays no design dividend — there is no `smelt.workspace.<other>` namespace the prefix disambiguates against, and the per-entity namespace shape matches the existing `smelt.columns_of` (narrow reflection per a specific `TableExpr`) more cleanly than a workspace-grouped namespace would. Option (ii) wins: `smelt.models.with_tag` reads as the rest of the language reads.

**Why the closed accessor set (`with_tag`, `all`) rather than a query DSL.** Alternatives considered were (i) a query-builder surface (`smelt.models.where(tag: "core").and(materialized: "table")`); (ii) lambda-based filtering as the only surface (`smelt.models.all() |> filter(fn m => m.tags |> any(fn t => t == "core"))`); (iii) a closed accessor set with named queries. Option (i) replicates `filter`-over-`all` with a parallel surface and requires a query-AST construct that pays no design dividend. Option (ii) is the *implementation* of `with_tag` — a user can always write the equivalent — but `with_tag` is the common case, and shipping the named accessor keeps the surface terse. Option (iii) wins on the same closed-registry argument as reducers and `ColumnRef` fields: predictable surface, anchored diagnostics on misuse, room to add accessors when concrete demand surfaces.

**Why `ModelRef` and `SourceRef` are closed records with four fields.** The field set `{path, name, tags, columns}` is the minimum closure that makes the killer per-cohort union demo expressible end-to-end:

- `m.tags` — chain further filtering by additional criteria
- `m.columns` — drive `coalesce_numeric`-style HOF chains across all matching models
- `m.path` — diagnostics anchoring and LSP `model_origin` framing
- `m.name` — user-facing logging and goto-def UI

Adding more fields (a `materialization`, a `backends:` list, a `description`) requires a spec edit and is paced by examples that demand them. The closed-record discipline mirrors `ColumnRef`'s rationale.

**Why `SourceRef` shares `ModelRef`'s field set.** A source is a workspace entity with a path, name, tag set, and column list — the same four observables a model exposes for wide reflection purposes. Splitting the shapes (a smaller `SourceRef` lacking `columns`, or a `SourceRef` with source-specific fields like `connection`) was considered and rejected because every use case discovered so far (filter by tag, project to `TableExpr`, iterate column list) is identical between models and sources. Uniformity keeps the surface terse; divergence remains available as a future spec edit if a source-specific field surfaces concrete pressure.

**Why `ModelRef <: TableExpr` (subtyping) rather than an explicit `.table_expr` projection.** The chosen rule lifts `List<ModelRef>` to `List<TableExpr>` via the existing `List<T>` covariant-subtyping rule, so `smelt.models.with_tag('cohort') |> reduce(union_all)` typechecks without a `map(fn m => m.table_expr)` step. An explicit projection field was the alternative; rejected because the killer demo would read `smelt.models.with_tag('cohort') |> map(fn m => m.table_expr) |> reduce(union_all)`, which is ceremony with no payback (`m.table_expr` carries no information `m` doesn't already carry; the user never wants the projection without immediately consuming it as a `TableExpr`). The one-way subtyping ensures `ModelRef` values retain their workspace-identity fields (`path`, `name`, `tags`) until consumed at a splice point; the lift is invisible to the user and is governed by the same fragment-sort assignability rule that already handles `smelt.<path>` to `TableExpr` resolution. The same argument applies to `SourceRef`.

**Why path-sorted determinism rather than declaration order or topological order.** Source-file declaration order is unstable under workspace edits — renaming a file changes the order; an LSP refactor that reorders files changes it. Path-sorted order is stable under arbitrary edits except renames, and a rename is a user-visible operation that the user expects to change downstream model identities. Topological order (over the dep graph) was considered and rejected because `with_tag` queries return arbitrary subgraphs (or no graph at all if the tagged models have no inter-dependencies); imposing a topological order would couple wide reflection to the dep graph in ways that introduce surprising orderings when tags don't track dependencies. Path order is the dial that produces predictable, edit-stable lists.

### Records — design rationale

**Why both inline and named record types.** Research §4.10.2 considered three approaches: (i) inline-only structural records (`{field: Type}` at every annotation position); (ii) named-only nominal records (`smelt.record Name = {…}`, no inline syntax); (iii) both supported. Option (i) forces config-loader schemas to repeat themselves at every load site — a five-field schema loaded by three files reads as fifteen lines of duplicated structure, with no goto-definition target. Option (ii) makes one-shot loader schemas heavyweight — a single-loader, four-field config asks the user to introduce a workspace-global declaration. Option (iii) lets the user pick: inline for one-shot, named for recurrence. The cost — a marginally larger type-checker surface (two forms instead of one) — is paid once at implementation time; the user gets the shape that fits their use case.

**Why structural typing rather than nominal.** Two inline records with the same field set are the same type, and a named record is interchangeable with a structurally-identical inline record. The alternative — nominal typing, where `SourceEntry` and `{name: Text, columns: List<Text>}` are distinct types — was rejected because it would require every loader call site to refer to the named declaration even when an inline form is more readable. The named-record declaration adds metadata (a declaration site, a hover signature, a goto-def target) without minting a nominal type. This matches the research-doc framing: records are shapes, not nominal identities.

**Why width subtyping.** A record `{a: T, b: U}` is a subtype of `{a: T}` because the wider type carries everything the narrower one needs, and meta-language records are immutable. The classical objection to width subtyping (extra fields can be mutated through a narrower view) does not apply when values cannot be mutated. Width subtyping pays off at the killer demo: a `Map<Text, TenantConfig>` whose `TenantConfig` has six fields can be passed to a HOF whose lambda expects only `{schema: Text}`, without the user writing a `map(fn t => {schema: t.schema})` widening step.

**Why required-field enforcement at the literal.** A record literal omitting a target field emits `RecordFieldMissing`. The alternative — silently treating missing fields as `Unknown` — was rejected because record literals are typically authored against a schema (a loader schema, a `ModelDef` shape) and missing fields silently propagating produce surprising downstream behaviour. The required-field rule matches the dbt-exceedance argument: typed config catches missing-field errors at the literal, not at the use site.

**Why field-name matching is case-sensitive byte-equal.** Case-insensitive matching (`Schema = "..."` matching against a declared `schema:` field) was considered and rejected because it propagates into rename-refactor scope ambiguity (renaming `Schema` to `Database` does not silently absorb existing `schema:` keys, but case-insensitive matching would make this brittle). Byte-equal matching produces predictable error messages and matches every modern config-loader's behaviour (YAML, JSON, TOML keys are case-sensitive by spec). Users wanting `Schema` and `schema` to refer to the same field must write the declared name consistently at every literal site.

**Why no record-shape inference at the literal.** A `{a: 1, b: "x"}` literal with no inferable target type emits `RecordLiteralUnknownTarget`, not an inferred `{a: Integer, b: Text}`. The inferred form was considered but produces silent retypings under partial edits: if the user types `{name: "foo"}` intending to fill in more fields, the type checker would commit to `{name: Text}` and reject the next field added. The non-inference rule keeps record literals on the same bidirectional-checking discipline as `[]` empty lists — annotation required when no context supplies the target shape.

**Why reflection witnesses are forbidden as record field types.** A `smelt.record Bad = { c: ColumnRef }` declaration was considered and rejected because it would leak the reflection witnesses' internal representation into user-writable surface, and would invite users to construct `ColumnRef` literals (forbidden per the reflection invariants). The closed admissibility set for record field types is the user-writable subset of the meta-type vocabulary; reflection witnesses remain meta-only and accessible only through their dedicated reflection accessors.

### Maps — design rationale

**Why `Map<K, V>` at all.** Research §4.10.3 considered three options: (i) add `Map<K, V>` with the reduced API; (ii) skip `Map` and require lists-of-records; (iii) `Map<K, V>` as sugar for `List<Record<{key: K, value: V}>>`. Option (ii) forces config authors to restructure YAML — a naturally-keyed `tenants: { acme: {…}, globex: {…} }` becomes a list of records with `name:` repeating the key. Option (iii) is appealing (zero new type) but `m.get('missing')` would be O(n) linear search and would produce list-style "element not found" diagnostics rather than map-style "key not found" diagnostics. Option (i) wins: a dedicated type matches YAML's natural shape and supports key-style operations (`get`, `has`) with the right complexity and the right diagnostics.

**Why `K` is constrained to `Text` in v1.** YAML and JSON mapping keys are always strings in their native data models; admitting non-`Text` keys requires committing to a hash/equality discipline for `Integer`, `Date`, record-typed keys, etc., none of which has a concrete v1 use case. Restricting `K` to `Text` keeps the implementation a `BTreeMap<String, V>` or equivalent and keeps the LSP completion list (`m.get(<cursor>)` offering keys) trivially renderable. Users wanting integer keys can write `m.get("42")` with `K = Text` and pay the small ceremony; broader-`K` support is a future spec edit.

**Why no `Map` literal syntax.** Adding a `Map` literal form would require disambiguating it from record literals (both naturally use brace-delimited key-value syntax). The disambiguation rule would either rely on declared target type (which has its own brittleness) or introduce a sigil (`#{...}` à la Clojure, `Map { k => v }` à la Ruby) that costs user attention without payback. Since `Map` values originate from loaders in the common case, the v1 surface has no missing-feature pressure on a literal form. Concrete pressure (a use case for inline `Map` literals in source files) is the trigger for revisiting.

**Why method-call syntax for the Map API.** Research §4.10.3 sketched bare-function API (`entries(m)`, `keys(m)`, `get(m, k)`). Method-call syntax was preferred because: (i) the five names — `entries`, `keys`, `values`, `get`, `has` — are common identifiers users would expect to bind in their own `smelt.define` declarations; reserving them workspace-wide is a heavier discipline cost than the HOF-name reservation. Method-call syntax scopes them to `Map<…>` values, leaving the bare names available to users. (ii) Pipe composition still works: `m |> entries |> map(…)` is not valid because `entries` has no `(`, but `m.entries() |> map(…)` is the equivalent canonical form. The pipe spec rule "RHS must be a call" is unaffected.

**Why entries are sorted ascending by key.** Two alternatives: (i) preserve the load-file's textual order (the YAML mapping's appearance order); (ii) sort by key. Option (i) is the natural reading but is unstable across YAML serialisers (some emit alphabetised, others preserve authoring order), and is hard to teach as a determinism guarantee. Option (ii) is stable, easy to teach (`m.entries()` lists keys A-to-Z), matches `smelt.models.with_tag`'s path-sorted ordering rule, and matches `BTreeMap`'s natural iteration. The cost — losing authoring order — is recovered by the rare user who wants it via an explicit `m.entries() |> sort_by(fn e => e.original_index)` once `sort_by` ships (not in v1).

**Why `m.get(k)` errors loudly on missing keys.** Three options: (i) statically-known missing keys are `MapGetMissingKey`, value `Unknown`; (ii) statically-known missing keys are silently `Unknown` with no diagnostic; (iii) `get` returns `Optional<V>` always, forcing a `default(v) |> …` consumer. Option (iii) requires a sum-type-like `Optional<V>` mechanism that is out of scope. Option (ii) matches the dbt failure mode this work exists to fix — silent misses propagate to incorrect SQL. Option (i) is the strict default; the future `Optional` surface can soften it for users who want defaulting. The strict default is recoverable to soft semantics via `m.has(k) |> if then else` once the meta-language ternary lands; the soft default is not recoverable to strict.

**Why `Map<K, V>` is invariant in both axes.** Covariant `K` would admit unsound lookups (a `Map<NarrowKey, V>` typed as `Map<WiderKey, V>` accepts wider-key lookups with no binding, defeating static `MapGetMissingKey` detection). Covariant `V` is sound (no `Map` value mutation), but matching `K`'s invariance keeps the rule uniform and avoids users having to memorise that `K` is invariant while `V` is covariant. The cost — a `Map<Text, Integer>` cannot widen to `Map<Text, Number>` even when the LUB would be safe — is recovered by `m |> entries |> map(fn e => {key: e.key, value: e.value : Number})` if the user genuinely needs the widening. The simpler rule wins.

## Constraints & Invariants

### Meta-world invariants (always hold)

- **Meta evaluation never reaches the database engine.** Every meta value is consumed during type checking or codegen-time expansion. The DB-facing SQL contains only Data-World constructs.
- **`smelt-db/src/type_inference.rs` remains pure.** New HOF and reflection rules are added as pure functions; Salsa queries call them, they do not call Salsa queries. (See `CLAUDE.md` Pure Function Rule.)
- **Termination is structural, not check-and-error.** The grammar admits no construct that requires runtime fixed-point iteration. If a syntax extension would admit unbounded recursion, it is rejected at the spec level, not allowed and policed at evaluation time.
- **No expansion-frame regression.** Adding HOF and multi-model production must not weaken the `expansion.md` frame-stack contract. Diagnostics from inside a `map(xs, fn x => …)` body must surface with a frame stack that names `map` and the per-element index.
- **Bidirectional checking remains decidable.** New types interlock with existing widening rules without introducing non-deterministic checks.

### List and spread invariants

- **Lists are immutable.** No mutation operation. `map` / `filter` produce new `List<T>` values; the originals are unobservable as mutated.
- **Lists are finite.** Length is known at the moment of construction. The language admits no streaming, lazy, or infinite-list construct.
- **`SmeltType::List(Box<SmeltType>)` is the canonical meta-list witness.** The existing `SmeltType::SelectItems { kind, context }` does not become `List<…>` and is not retired. The two coexist; `SelectItems` remains the splice-context-bearing form for SELECT lists. The `List<T>` ↔ `SelectItems<…>` bridge is reducer territory.
- **List literals admit no implicit meta-to-data lift on their elements.** The spread operator passes meta-list elements into Data-World grammar slots without changing their kind; meta-`Text` lifts to a SQL identifier only at the four enumerated identifier-lift positions (see §"Meta-`Text`-as-identifier lift").
- **`...` token is exclusive to spread.** The lexer reserves `...` for spread; it is not used by any other grammar construct. Future extensions may extend its use only within the spread family (e.g. row-tail markers `..r` in `Struct<{…}>` are a separate token spelled with two dots, already in use).

### Lambda and HOF invariants

- **Lambdas have no first-class surface.** `Lambda<T, U>` is a meta-only type whose values are constructed only at HOF positional argument positions and consumed only by the corresponding HOF. The type does not appear in user-writable annotations, in `smelt.define` parameters / return types, in record fields, in list elements, or in named-argument values.
- **HOFs are pure functions of their inputs.** `map`, `filter`, and `reduce` produce a result fully determined by `(xs, f|p|r)` — no clock, no random, no hidden state. Re-evaluation under the same inputs produces the same result, byte-equal at the CST level for codegen-time expansion.
- **HOF and reducer names are workspace-wide reserved.** No `smelt.define` declaration, lambda parameter, or other meta-namespace identifier may bind these names. The reservation is part of the closed-registry contract.
- **HOF inline-expansion frames carry no `fn_id`.** The anonymous-frame form registered in `expansion.md`'s anonymous-frame contract is the only addition to the frame-stack contract for HOFs; HOF lambda bodies are not declarations and have no per-function identity. The frame's `function` field carries the HOF name; producers must populate `call_site_range` and the optional `element_index`.
- **`SmeltType::Lambda` is invariant in its parameters.** `Lambda<S, T>` and `Lambda<S', T'>` unify only when `S = S'` and `T = T'`. No subtyping rule applies. The HOF's type-checking rule binds the lambda's parameter and synthesises its return; it does not need lambda subtyping.

### Pipe invariants

- **Pipe is parser-level desugaring.** `|>` is rewritten to a call-form CST node before type checking; no type-system rule, evaluation rule, or LSP feature observes a "pipe value" — every downstream layer sees the equivalent un-piped call. Reverting a pipe to its un-piped form is a mechanical refactor and never changes semantics.

### Reducer invariants

- **Reducer registry is closed.** The seven reducers in §"Contextual reducers" are the entire set. Adding one requires a spec edit and a compiler change. User code may not introduce a reducer; user code may not pass an arbitrary value as `reduce`'s second argument (the second argument is parsed as a bare reducer identifier, not a value-bearing expression).
- **`Reducer<T>` is not a user-writable type.** The internal type-system witness for reducer identifiers is unspeced at the user surface. The surface presents reducers as bare identifiers with closed-registry membership; future user-defined reducers would have to surface a `Reducer<T>` type, but that is post-plan.

### Compile-time variable invariants

- **`smelt.config.var` is literal-only.** The argument is constrained to be a string literal. Expression-valued lookups require loader-family integration and are a deliberate exclusion at this layer.

### Reflection invariants

- **`smelt.columns_of` is a Salsa-cached pure function of workspace state.** The accessor performs no I/O, observes no clock or random source, and re-evaluates byte-equal across runs on the same workspace input.
- **`ColumnRef` is a closed record.** The field set is exactly `{name: Text, type: DataType, is_numeric: Boolean}`. Adding a field requires a spec edit and a compiler change.
- **`ColumnRef` is not user-constructible.** Values originate only from reflection accessors (`smelt.columns_of` and the wide-reflection accessors). User code cannot construct a `ColumnRef` literal; the internal `SmeltType` witness is not part of the user surface.
- **`ColumnRef` has no first-class user-writable surface.** The type is not a writable `smelt.define` parameter or return type, not a list element type users construct in literals, and not a YAML-loadable record. The user-writable record surface does not retroactively expose `ColumnRef`'s structure.
- **Reflection results preserve source ordering.** `smelt.columns_of(t)` returns columns in the order they appear in `t`'s schema declaration. Order is observable by users; reordering would invalidate `coalesce_numeric`-style HOF chains that depend on positional column behaviour.
- **Body-check time produces parametric `List<ColumnRef>`; expansion time materialises the concrete list.** This invariant matches `expansion.md`'s body-walk-with-bound-parameters rule and is the load-bearing decision that lets reflection ship without per-call-site re-checking of function bodies.
- **Field-projection diagnostics anchor at the field span.** `ColumnRefFieldUnknown` reports at the offending `c.<bad>` field token, not at the lambda parameter or the source list expression.

### Identifier-lift invariants

- **Meta-`Text`-as-identifier lift is narrow and grammar-position-driven.** Lift applies in exactly the four enumerated positions (column-reference, AS-alias, ORDER BY column-reference, GROUP BY column-reference); in any other position a meta-`Text` retains its `Text` value. The lift surface is the dial that may widen under concrete demand; widening requires a spec edit with explicit additions to the position table.
- **Identifier lift is meta-only.** Runtime `Expr<Text>` values cannot lift to identifiers; only compile-time-known meta-`Text` values lift. This preserves the invariant that SQL identifiers are parse-time identifiers, not runtime values.

### Record invariants

- **Record types are workspace-globally unique under `smelt.record` names.** Two declarations of the same name across any pair of files emit `SmeltRecordRedefinition`; uniqueness is enforced at workspace-level type-context construction.
- **Record values are immutable.** No record mutation operation. A record literal produces a fresh record value; field projection synthesises a new value of the field type. Width subtyping's soundness depends on this invariant.
- **Record types form a DAG.** A field type may reference any record name declared earlier in the workspace's evaluation order; cyclic declarations (direct or mutual) emit `RecordCyclicDeclaration`. The acyclic structure makes type-check termination structural.
- **Record literals are bidirectionally type-checked.** A literal in a position with no inferable target type is a type error (`RecordLiteralUnknownTarget`); the type checker never synthesises an inline record type from a literal alone. This matches `[]` empty-list rules and protects against silent retypings under partial editing.
- **Reflection witnesses are not user-writable record field types.** `ColumnRef`, `ModelRef`, `SourceRef` are admissible inside the meta-language vocabulary but excluded from `smelt.record` field types; `RecordFieldTypeForbidden` enforces the exclusion.
- **Records are pure meta-world values.** Record values do not reach the database engine; record bindings in Data-World positions emit `RecordInDataWorld`. Record fields holding fragment-sort values (`TableExpr`, `Expr<T>`) consume their projection at splice points per the standard splice rules.

### Map invariants

- **`Map<K, V>` keys are `Text` in v1.** The constraint is enforced at the type expression; `MapKeyTypeNotText` anchors at the offending key-type expression. Future extensions revisiting this constraint must commit to an equality/hashing surface for the new key type.
- **`Map<K, V>` values originate only from loaders.** No literal syntax in v1; no in-language map construction. Adding a producer requires a spec edit.
- **Map iteration is byte-lexicographic by key.** `m.entries()`, `m.keys()`, `m.values()` produce lists in ascending key order. The order is observable; HOF chains over `m.entries()` see entries in this sorted order. Reordering would invalidate predicable downstream behaviour.
- **`Map<K, V>` is invariant in both axes.** No subtyping rule applies between `Map<Text, Integer>` and `Map<Text, Number>` (or any pair). Width subtyping over record-typed `V` is recovered through `entries`-projection-style transformations, not through `Map`-level variance.
- **`m.get(k)` is strict by default.** A statically-known missing key is a diagnostic, not a silent `Unknown`. The strict behaviour catches authoring bugs at the loader-call site; softening to default-on-missing requires a future `Optional`/`if-then-else` surface.
- **Map API is closed.** The five methods (`entries`, `keys`, `values`, `get`, `has`) are the entire surface; misuse emits `MapApiUnknown`. Future methods require a spec edit and a compiler change.
- **Maps are pure meta-world values.** Map values do not reach the database engine; consumers transform a `Map` to a `List` via the entries/keys/values projections and consume the list at splice points.

### Out-of-scope by deliberate choice

- **Pipe-SQL extension** (research §4.6 alternative b) — porting the pipe operator into Data-World queries is a separate paper.
- **Tuples** — rejected in favour of records; `zip_with` (if shipped) takes a multi-arg lambda rather than producing a `List<Tuple<…>>`.
- **Generators-of-generators** — multi-model production forbids one generator file consuming another generator's output. Cycles in workspace-shape evaluation are rejected at the spec level.
- **Heterogeneous lists / sum types** — meta lists are monomorphic. A list with mixed element types is a type error; sum types are out of scope.
- **User-defined reducers** — the reducer registry is closed. Extension requires a compiler change (revisit when concrete pain emerges and a soundness-verification approach exists).
- **`infer_schema` codegen mode** — schema authoring for config loaders is required; tools that infer schemas from sample data are post-plan.
- **`Map` literals** — no in-source `Map<K, V>` literal syntax; values come from loaders. Concrete demand for inline maps is the trigger for revisiting.
- **Non-`Text` `Map` keys** — `Map<Integer, V>`, `Map<Date, V>`, record-typed keys deferred until a concrete equality/hashing surface is needed.
- **`Optional<T>` and record-shape inference** — `m.get(k)` returns `V` strictly; record literals require explicit target types. Both soften to ergonomic alternatives only when a separate spec defines `Optional<T>` and the surrounding consumer pattern.
- **Mutually recursive records** — record declarations form a DAG; a future spec can lift the restriction when a use case demands it.

## Known Divergences / Open Questions

- **Reflection (`smelt.columns_of`, `ColumnRef`, identifier lift) is not yet implemented.** The surface and semantics above are normative; the implementation, the four reflection diagnostic codes, the `column_origin` extension to the anonymous expansion frame, and the LSP hover/goto/completion paths have not yet landed. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Cross-spec touches required for reflection.** The reflection implementation must land two adjacent-spec touches: (i) `expansion.md`'s `column_origin` field on the anonymous-frame contract; (ii) `lsp.md`'s reflection LSP obligations (hover/completion/goto-def for ColumnRef field projection and lifted identifiers). `schema_evolution.md` records (informationally, not normatively) the implication that a column added to a source must propagate to `smelt.columns_of`-sourced HOF outputs; this is observable behaviour falling out of expansion-time evaluation, not a separate behavioural change. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Records, `Map<K,V>`, and schema-typed loaders are implemented.** The record, map, and loader surfaces above are normative and landed. Residual divergences — recursive schemas, per-key deep-merge for `Map<Text, S>` overlays, and `Optional<V>` schema fields — remain deferred per `meta_config_loading.md`. Tracked in `docs/plans/20260509-meta-language-E1.md`.
- **Map iteration order and `m.get` strictness are committed.** Two design dials closed in §"Maps — design rationale": (i) `m.entries()` / `m.keys()` / `m.values()` produce ascending byte-lexicographic order on keys; (ii) `m.get(k)` on a statically-known missing key is `MapGetMissingKey`, not a silent `Unknown`. Future softening (insertion-order iteration, `Optional<V>` return type) requires a separate spec edit.
- **Multi-model production not yet implemented.** Will define the `generates: models` frontmatter directive; the `ModelDef` record meta type; the meta-`Text`-as-identifier lift in path positions; and the workspace-shape change ("one file may produce N models"). Mechanism is not yet finalised — research §4.10.4 leans on a frontmatter directive plus a body returning `List<ModelDef>`. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Identifier-lift positions in path contexts undecided.** Multi-model production must commit to which SQL grammar slots admit a meta-`Text` lift (model path components, column aliases, CTE names) and which do not (arbitrary keywords). The narrowness is what keeps the lift from drifting into Jinja territory. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Polish (parameterised reducers, multi-arg lambdas, ternary, `zip_with`) not yet implemented.** Will define parameterised reducers (e.g. `concat_with(sep)`); multi-arg lambdas; the meta-world ternary `if cond then a else b`; and `zip_with` if any shipped example demands it. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **LSP completeness work not yet implemented.** Rename support for new constructs and guaranteed hover/goto-def/completion/diagnostics-with-frame-stacks across every shipped meta-language surface element have not yet landed. (No new syntactic surface; LSP capability is part of the spec because the user-visible behaviour of editor tooling is part of "how this feature works".) Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **`Array<U>(…)` runtime-array constructor.** §"Per-construct semantics — Lists and spread" rule 3 references `Array<U>(…)` as the explicit opt-in for the runtime-array reading of `[…]`. The constructor is not yet implemented; until it lands, the only Data-World path to a runtime array is the existing `[1, 2, 3]` literal in an `Expr<Array<U>>` position (governed by `types.md`). Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Lambda surface is `fn x => body`.** Position-based disambiguation (research §4.5 backup) is not part of the surface.
- **HOF expansion frames are anonymous.** Producers populate the `function` field with the HOF name, but the frame has no `fn_id` (HOFs are built-ins, not user-defined functions). The LSP renderer reads only `call_site_range`; the per-element-index field is producer-side until a renderer follow-up surfaces it. Tracked here so future planner / renderer work preserves the contract.
- **`Reducer<T>` is not a user surface.** The closed registry presents reducers as bare identifiers; the type-system witness behind them is internal. Future user-defined reducers would require a `Reducer<T>` user-writable type and a soundness-verification approach (associativity, identity); both are post-plan.
- **Lifted-identifier hover and goto-def Backend dispatch not yet wired.** `hover_text_for_lifted_identifier` and `goto_def_for_lifted_identifier` are implemented as pure helpers (no Salsa dependency), but the LSP dispatch in `Backend::hover` and `Backend::goto_definition` does not yet detect when the cursor is inside one of the four lift positions (column-reference, AS-alias, ORDER BY, GROUP BY) and route to these helpers. Full wiring requires distinguishing a `c.name` field-projection (which uses the existing ColumnRef field-hover path) from a `c.name` expression used as an SQL identifier (the lift case); this distinction requires parent-AST-context analysis not yet implemented in the pure dispatch layer. The `goto_def_for_lifted_identifier` helper also returns `None` in v1 because the `source_span` field on `ColumnRefValue` is `Option<TextRange>` without a file path, making PathBuf construction impossible without Salsa context. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **`ColumnRef.type` field projection currently returns `Unknown`.** The closed three-field set for `ColumnRef` is `{name: Text, type: DataType, is_numeric: Boolean}` per §Semantics rule 4. The type checker recognises `c.type` as a valid field access and emits no `ColumnRefFieldUnknown` diagnostic, but maps the result to `Unknown` rather than a `DataType` meta literal. Equality comparisons such as `c.type == Integer` therefore silently degrade — the comparison type-checks as `Unknown` rather than `Boolean`, so predicates that depend on it do not filter as intended. The richer `DataType` meta-literal surface, which is needed to give `c.type` a non-`Unknown` return type, lands with the wider record and data-literal work. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **`ModelRef` / `SourceRef` goto-def at splice sites is a graceful no-op.** `goto_def_for_model_ref_value` and `goto_def_for_source_ref_value` are implemented as pure helpers that pass through a supplied source path. The Backend dispatch in `Backend::goto_definition` does not yet detect when the cursor is on a `ModelRef` (resp. `SourceRef`) value at a `FROM`-clause or reducer splice site and resolve the underlying model's source `.sql` file (resp. source YAML) — concrete path resolution requires expansion-time context that the pure-helper layer cannot reach. Until the Backend dispatch is wired, goto-def on such splice sites returns a graceful no-op (no panic, no incorrect navigation). Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Lift-scope validation at body-check time is suppressed; expansion-time validation is not yet wired.** Spec §Semantics rule 6 locates `UnknownColumn` validation for a lifted identifier at *expansion time*, after the per-element column name is known. At body-check time the structural lift is recognised (so no spurious `UnknownIdentifier` is emitted for the `c.name` expression) but no scope check is performed. Expansion-time validation — which would catch a lifted column name that does not exist in the call-site schema — has not yet been wired into the expansion path. Until it is, a `c.name` lift that references a non-existent column will silently produce incorrect SQL rather than a diagnostic. Tracked in `docs/plans/20260509-meta-language-overall.md`.

## References

- **Code**:
  - `crates/smelt-parser/src/lexer.rs` — `LBRACKET`, `RBRACKET`, `DOTDOTDOT`, `FN`, `PIPE_ARROW` tokens (lex `||` before `|>` to avoid mis-tokenisation).
  - `crates/smelt-parser/src/parser.rs` — `ARRAY_LITERAL` (reused for list literals), `LIST_SPREAD`, `LAMBDA`, `PIPE_EXPR` productions; lowest-precedence left-associative `|>`; `RHS-must-be-call` validator producing `PipeRhsNotCall`.
  - `crates/smelt-parser/src/ast.rs` — typed wrappers for the new CST nodes.
  - `crates/smelt-types/src/signatures.rs` — `SmeltType::List(Box<SmeltType>)`, `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` variants; meta-only `ColumnRef` `SmeltType` witness (the spec leaves the exact variant shape — dedicated `ColumnRef` variant vs internal `Record` instantiation — to the implementation, subject to the closed-field invariant); user-writable `SmeltType::Record { fields, name: Option<String> }` variant (structural by field set; the optional name is hover-only metadata, not a nominal distinguisher) and `SmeltType::Map(Box<SmeltType>, Box<SmeltType>)` variant (invariant in both axes); workspace-level `SmeltRecordDeclaration` registry keyed by `smelt.record` name with declaration spans.
  - `crates/smelt-db/src/type_inference.rs` — pure inference for list literals and spread (LUB, covariant subtyping, empty-literal handling); HOF dispatch (bidirectional binding of lambda parameter type from HOF `T`); reducer dispatch (closed-registry lookup, input-type validation, empty-list identity); pipe desugaring at AST level; `smelt.columns_of` (synthesises `List<ColumnRef>` from a `TableExpr` argument); `ColumnRef` field projection (closed lookup against the field set); meta-`Text`-as-identifier lift detection at the four enumerated grammar positions; record literal bidirectional checking (target-type-driven field validation, width-subtyping assignability); record field projection (closed-set lookup against the target's declared fields); `Map<K, V>` method-call dispatch (closed `{entries, keys, values, get, has}` registry, statically-resolvable-key `get`/`has` evaluation).
  - `crates/smelt-db/src/function_body_check.rs` — anonymous-frame stamping at HOF call sites; lambda parameter scoping in body walks; `column_origin` extension on the anonymous expansion frame; per-element provenance stamping for `columns_of`-sourced HOF iterations; expansion-time materialisation of `List<ColumnRef>` from a resolved `TableExpr` schema; record-literal validation at body-walk time (required/duplicate/unknown field detection); `map_origin` extension on the anonymous expansion frame for HOF chains sourced from `m.entries()`.
  - `crates/smelt-db/src/lib.rs::DiagnosticCode` — every diagnostic code listed under §Surface (lists, lambdas, pipe, reducers, compile-time variables, reflection, records, maps).
  - `crates/smelt-db/src/lib.rs` — closed reducer registry (`REDUCER_REGISTRY`); `smelt.config.var` resolver query against `smelt.yml` `vars:`; `smelt.columns_of` Salsa query (resolves source schema via existing `ModelSchema` machinery); closed `COLUMN_REF_FIELDS` registry; closed `MAP_API_METHODS` registry; workspace-level `smelt_record_declarations()` Salsa query indexing every `smelt.record` declaration in the workspace, used for goto-def and redefinition detection.
  - `crates/smelt-parser/src/{lexer,parser,ast}.rs` — `LBRACE`, `RBRACE`, `COLON` already exist for SQL grammar; reuse for record literals and inline record types. New parser productions: `SMELT_RECORD_DECL` (top-level `smelt.record Name = { fields }`), `RECORD_LITERAL` (brace-delimited key-value list at value positions), `RECORD_TYPE_INLINE` (brace-delimited typed field list at type-annotation positions), `MAP_METHOD_CALL` (post-dot method-call dispatch on `Map<…>`-typed expressions).
  - `crates/smelt-lsp/src/lib.rs` — hover for list/spread, lambdas, HOF calls, pipe expressions, reducer names, `smelt.config.var`, `smelt.columns_of`, ColumnRef field projection, lifted identifiers, record-typed bindings, record literal opening brace, record field projection, `Map<K, V>`-typed bindings, Map API methods; goto-def for lambda parameters, `smelt.config.var` arguments, lifted identifiers, `smelt.record` declarations, record-literal field names; completion in lambda bodies, reducer-argument positions, ColumnRef field set, `smelt.columns_of` argument positions, record-literal field positions (offering unfilled target fields), record-field projection (offering declared field list), `m.<cursor>` (offering closed Map API), `m.get(<cursor>)`/`m.has(<cursor>)` (offering statically-known keys).
- **Tests**:
  - `crates/smelt-parser/src/{lexer,parser}.rs::tests` — token, production, and error-recovery cases for `[…]`, `...`, `fn`, `|>`, `LAMBDA`, `PIPE_EXPR`; multi-arg-lambda parser error; pipe-rhs-not-call recovery; `smelt.record Name = { fields }` top-level declaration; record literal `{f: v, …}` at value positions; inline record type `{f: T, …}` at annotation positions; Map method-call (`m.entries()` etc.) parser productions.
  - `crates/smelt-db/src/type_inference.rs::tests` — list literal LUB, empty-literal target inference, spread evaluation, forbidden positions; HOF dispatch, lambda parameter binding, reducer input-type checking, empty-list identity, pipe desugaring; `smelt.config.var` resolution and YAML scalar coercion; `smelt.columns_of` argument-type checking (TableExpr-only); `ColumnRef` field projection (closed-set lookup); lift-position grammar checks (the four enumerated positions accept; all others reject); lift narrowness rejection cases (runtime `Expr<Text>` in identifier position remains a Data-World type error); record literal bidirectional checking (required field, duplicate field, unknown field, type-mismatch, no-target diagnostics); record field projection (closed-set lookup, mid-chain projection on non-record value); width subtyping (`{a: T, b: U}` assignable to `{a: T}` but not reverse); `smelt.record` redefinition detection; record-cycle detection; Map API closed-set dispatch (every method, every arity error, every diagnostic code); statically-known `m.get(k)` present/absent paths; `MapKeyTypeNotText` at non-Text key declarations.
  - `crates/smelt-db/src/function_body_check.rs::tests` — anonymous-frame stamping; multi-frame chains crossing a HOF; lambda parameter scoping under `TableExpr` parameters; `column_origin` frame stamping for `columns_of`-sourced HOF lambda bodies; expansion-time materialisation of `List<ColumnRef>` from a resolved schema; `ColumnsOfUnresolvableSchema` recovery (drop-on-error); per-element provenance through the lift; record-literal validation under HOF body walks; `map_origin` frame stamping for HOF chains sourced from `m.entries()`.
  - `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/meta_lists/`, `examples/meta_hofs/`, `examples/meta_columns/`, `examples/meta_workspace/`, `examples/meta_config/` acceptance gates.
- **User docs**:
  - `docs-site/docs/meta-language/index.md` — overview of the meta-language.
  - `docs-site/docs/meta-language/lists.md` — `List<T>`, list literals, spread.
  - `docs-site/docs/meta-language/hofs.md` — `map`, `filter`, `reduce`.
  - `docs-site/docs/meta-language/lambdas.md` — `fn x => body` surface and scoping.
  - `docs-site/docs/meta-language/pipes.md` — `|>` operator.
  - `docs-site/docs/meta-language/reducers.md` — closed reducer registry and empty-list identities.
  - `docs-site/docs/meta-language/config-vars.md` — `smelt.config.var` lookups.
  - `docs-site/docs/meta-language/reflection.md` — `smelt.columns_of`, `ColumnRef`, the closed field set, the four-position identifier lift, the `coalesce_numeric` worked example; also covers `smelt.models.*` / `smelt.sources.*` wide reflection, `ModelRef` / `SourceRef`, and the union-by-tag worked example.
  - `docs-site/docs/meta-language/records.md` — `smelt.record` declarations, inline record types, record literals, field projection, width subtyping.
  - `docs-site/docs/meta-language/maps.md` — `Map<K, V>` type, the closed `{entries, keys, values, get, has}` API, missing-key diagnostics, iteration order.
  - `docs-site/docs/meta-language/reference.md` — alphabetical reference covering every HOF, reducer, `smelt.config.var`, `smelt.columns_of`, `ColumnRef`, `smelt.models.*` / `smelt.sources.*`, `ModelRef`, `SourceRef`, `smelt.record`, `Map<K, V>` API, and the lift positions table.
- **Plans (history)**:
  - `docs/plans/20260509-meta-language-overall.md` — meta-plan tracking the meta-language work
  - `docs/plans/20260509-meta-language-A.md`
  - `docs/plans/20260509-meta-language-B.md`
  - `docs/plans/20260509-meta-language-C.md`
  - `docs/plans/20260509-meta-language-D.md`
  - `docs/plans/20260509-meta-language-E1.md`
- **Related specs**:
  - `docs/specs/functions.md` — `smelt.define`, fragment sorts, named arguments (parser disambiguation surface)
  - `docs/specs/types.md` — `DataType` vocabulary, fragment-sort grammar, strict-by-default doctrine
  - `docs/specs/expansion.md` — codegen-time expansion, frame stacks, `Caller`/`Callee`/`Synthesized` provenance — extended by HOFs (anonymous-frame contract) and multi-model production
  - `docs/specs/scoping.md` — body scoping, splice contexts, parameters-first; lambda parameter scoping and generator-file scoping plug in here
  - `docs/specs/architecture.md` — `smelt.<path>` resolution, project layout — multi-model production amends the "1 file = 1+ models" invariant
  - `docs/specs/gradual_typing.md` — `Unknown` widening — `List<Unknown>` rules
  - `docs/specs/meta_config_loading.md` — file-loader family for `smelt.config.load_yaml` etc.
  - `docs/specs/model_selection.md`, `incremental_models.md`, `python_models.md`, `data_catalog.md`, `schema_evolution.md`, `cli.md`, `datagen.md` — multi-model production cross-feature touches
  - `docs/specs/lsp.md` — LSP support obligations
- **Research**:
  - `docs/research/20260507-typed-meta-programming.md` — design oracle: framing, alternatives at every choice point, sequencing, worked examples, open questions
  - `docs/research/20260413-smelt-functions.md` — parent paper for fragment sorts and `smelt.define`
