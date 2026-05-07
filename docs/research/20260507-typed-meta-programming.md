# Typed Dynamic Models: A Statically Analysable Meta-Language for smelt

**Date:** May 2026
**Status:** Research / Design Exploration
**Author:** Andrew Browne, with design input from Claude

This paper explores how smelt can match — and exceed — dbt's ability to express *dynamic models* (models whose shape, column list, or row source is computed from project metadata at compile time) while preserving the language's defining property: every program is statically analysable, every cross-reference is resolvable, every error has a source span. dbt achieves dynamism through Jinja templating; that choice forfeits typing, navigation, and LSP feedback inside macros. smelt's `smelt.define` functions cover *fragment-level* reuse with full static analysis but stop short of the dbt scenarios that depend on **iterating over project metadata** (columns, models, tags, sources). This paper proposes the building blocks of a user-visible meta-language — typed lists, list literals, a spread operator, higher-order functions, lambdas, a pipe operator, contextual reducers, and (eventually) reflection — and discusses alternatives at each level of detail. The unifying claim: smelt already has a meta-world (fragment sorts, two-tiered expansion, splice contexts); making it user-visible is a layering exercise on top of existing machinery, not a new language inside the language.

## 0. Authoring Context

> **For a continuing reader** (the author or any reviewer picking this up in a fresh session). This section is not part of the design — it captures provenance and confidence so the next person can pick up the thread without re-deriving what's settled.

**Origin.** First draft on 2026-05-07, out of a brainstorming conversation with Claude. The starting framing came from a proposal exchanged with Gemini covering: (a) HOFs (`map`/`filter`/`reduce`) with contextual reducers (`and_all`, `comma_sep`, `union_all`); (b) the pipe operator `|>`; (c) "Syntactic Lifting" / Meta-World vs Data-World; (d) a spread operator `...`. Andrew asked for full-scope research-paper treatment, not a spec, with alternatives at each level of detail.

**Status.** Research / design exploration. Nothing here is committed surface or behaviour. The §9 sequencing is *illustrative*; the §10 recommendation is *advisory*. When this hardens, the spec target proposed mid-conversation was `docs/specs/meta_language.md`.

**Sequencing instinct from Andrew, recorded for continuity.** Andrew floated "literal lists first" as the smallest slice that exercises the meta/data lifting mechanism, before HOFs and before reflection. §9 Phase A captures that. He explicitly wants alternatives kept open, not collapsed prematurely.

**Highest-leverage open decisions** (mirrored in §10):

- **Lambda syntax** — the `=>` collision with named-argument syntax has no costless fix. Current lean is keyword-prefixed `fn x => body`; backup is position-based disambiguation. See §4.5.
- **Reflection API shape** — sketched only in §4.8. Type *shape* (`ModelRef`, `ColumnRef`, `Schema`) is what needs committing now so the meta-language type system accommodates reflection without a breaking change later.

**Author confidence (subjective — for a reviewer's calibration):**

- **High** — §3 (meta-world recap), §6 (LSP exceedance over dbt), §8 (open-question enumeration).
- **Medium** — §4.1 (`List<T>`), §4.4 (HOF set), §4.6 (pipe), §4.7 (reducer registry).
- **Low** — §4.5 (lambda surface — `fn` is a guess), §4.8 (reflection API — sketch only), §5.2 / §5.5 (worked examples that lean on undecided sub-questions, especially the meta-`Text`-as-identifier lift).

**Consciously out of v1 scope** (per §1.2 and §8): hooks / lifecycle events, custom materialisations, pipe-SQL extension into the data world, meta-let / `@variables`, `Map<K, V>`, tuples, recursive reflection, user-extensible reducers.

**Background a continuing reader should pre-read** before extending this paper:

- `docs/specs/functions.md` — `smelt.define`, fragment sorts, `PASSING` clauses; what already exists in the meta-world.
- `docs/specs/expansion.md` — two senses of expansion, `Caller`/`Callee`/`Synthesized` provenance, frame-stack contract. The proposal here extends the frame-stack contract into HOF / lambda calls.
- `docs/specs/types.md` — `DataType` vocabulary, fragment-sort grammar, strict-by-default doctrine, the existing bidirectional checking machinery this paper relies on for §4.2 disambiguation.
- `docs/specs/scoping.md` — parameters-first resolution, splice contexts, no-overlap rule. Lambda capture rules in §4.5 must respect these.
- `docs/research/20260413-smelt-functions.md` — the parent paper. This proposal is a layered extension of the fragment-sort design described there; it does not replace any of it.

**Stylistic constraints worth honouring in any continuation:**

- Specs in this project carry design rationale, not just surface and rules. If/when this becomes a spec, the Design section needs to record what was rejected and why.
- Smelt prefers to defer features until concrete pain emerges. The §9 sequencing leans on this — don't ship reflection before the demand for it is felt.
- Naming and identity should fall out of physical structure where possible (the `smelt.<path>` doctrine). Reflection accessors in §4.8 should align with this when they harden.

## 1. The Problem — Beyond Functions

`smelt.define` (research §`20260413-smelt-functions.md`) closed the gap on **fragment reuse**: predicates, expressions, table transformers, and select-list shapes can be parameterised, called by name, and inlined with full type checking. What it does not address is the class of dbt scenarios where the *input to the SQL* is computed from the project itself.

A non-exhaustive catalogue of dbt patterns smelt cannot express today:

| Pattern | dbt idiom | What's needed |
|---|---|---|
| Union all models matching a tag | `{% for m in graph.nodes if 'audit' in m.config.tags %}` | Workspace reflection + list-of-table operations |
| Coalesce all numeric columns | `{% for c in adapter.get_columns(ref('x')) if c.is_numeric %}` | Schema reflection + filter + reduce |
| Per-environment dimension list | `{% set dims = var('dims') %}{% for d in dims %}` | Compile-time variable lookup + iteration |
| Pivot over a static category list | `{% for cat in ['a', 'b', 'c'] %}MAX(CASE WHEN ...) {% endfor %}` | List literal + map → SelectItems |
| Generic test parameterised over columns | `{% for col in cols %}check {{ col }} {% endfor %}` | List parameter + map |
| Surrogate key over column list | `dbt_utils.generate_surrogate_key(['a', 'b', 'c'])` | List parameter + reduce-with-separator |
| Cross-source schema enumeration | `{% for src in graph.sources %}` | Source reflection |

The shared shape is: a **list of metadata items** is computed at compile time, transformed through `map`/`filter`, and reduced into a SQL fragment that is spliced into a model. Jinja makes this textual. smelt should make it typed.

### 1.1 What "exceed dbt" means concretely

The bar is not just feature parity. Because dbt's macros compile through Jinja string-substitution, dbt cannot:

- Type-check anywhere inside a `{% for %}` body.
- Resolve `{{ col.name }}` to a definition the LSP can jump to.
- Show a hover for a macro-generated column.
- Anchor errors to the source span the user wrote inside a macro (errors surface from the post-substitution SQL).
- Refactor (rename) a column referenced through a macro.
- Optimise across a macro boundary.

A typed meta-language can do all of these — every meta-world value has a type, every map/filter/reduce has a known input/output shape, every diagnostic can carry an expansion-frame stack into the meta layer (the same machinery `expansion.md` already specifies for `smelt.define`).

### 1.2 What is *not* in scope

Out-of-scope for this paper (each is its own design exercise):

- **Hooks / lifecycle events** (dbt's `on-run-start`, `pre-hook`, `post-hook`). These are an orchestration concern, not a meta-language concern.
- **Custom materialisations** as a user-extension point. smelt's planner-rule API will eventually serve this need; that is a separate spec (`planner_integration.md`).
- **Custom dialects via macros**. smelt's `smelt.extern` + `backends:` frontmatter solves the dialect-portability axis differently.
- **Imperative control flow** (`if`/`else` blocks at the top level of a model). The proposal here is purely *value-level meta computation* — function-style, no statements.

## 2. Design Constraints

The proposal is constrained by smelt's load-bearing properties:

1. **Static analysability.** Every program must be type-checkable in a single pass without execution. The meta-language must have its own (small) type system that interlocks with the existing `DataType` and fragment-sort vocabularies.
2. **LSP-grade ergonomics.** Goto-definition must work through every meta call. Hover must show a meaningful type. Diagnostics must point at the source span the user wrote, not at a generated string.
3. **No string templating.** Meta values are CST nodes (or values that produce CST nodes). The compiler never re-parses output.
4. **Termination by construction.** Same property `smelt.define` enforces — no unbounded recursion, no halting problem at compile time. Meta evaluation is total.
5. **Composes with existing fragment sorts.** `Expr<T>`, `TableExpr`, `SelectItems<Kind, ctx>`, `OrderSpec` already exist and are used. The meta-language must add to them, not replace them.
6. **Compile-time only.** Meta-world values evaporate after expansion — they never reach the database engine. (Runtime arrays and structs are a separate, unrelated SQL feature in the `Array(T)` / `Struct({...})` `DataType` family.)

These five constraints filter the design space heavily. They rule out, for example, an embedded Python interpreter (constraint 1 fails), a separate DSL with its own parser (constraint 5 fails), and `eval`-style string macros (constraints 1 and 3).

## 3. The Meta-World, Already There

A mental shift is useful before discussing surface syntax: **smelt already has a meta-world**. It is just not user-visible.

When a user writes `smelt.define sum_of(parts: SelectItems<Scalar, base>)`, the parameter `parts` has a meta-world type (`SelectItems<Scalar, base>`). Its value lives at compile time. It produces SQL via splicing. The same is true for `Expr<T>` parameters, `TableExpr` parameters, `OrderSpec` parameters. The codegen-time expansion described in `expansion.md` § "Two senses of expansion" *is* a meta evaluation — it walks a CST in a context where parameters are bound to caller arguments and produces a fresh CST.

What this paper proposes is to **lift** that meta-world to user-visible status: give it a name (the Meta-World), give it primitive values (lists, scalars), give it operations (HOFs, pipe, spread), and give it a place in the type system (`List<T>`).

### 3.1 Meta-World vs Data-World

The two-tier model:

- **Data-World** is the SQL the database engine sees. Its types are the `DataType` vocabulary (`Integer`, `Text`, `Array(T)`, `Struct(...)`, etc.). Its values exist at query runtime.
- **Meta-World** is the compile-time computation that produces Data-World SQL. Its types are fragment sorts (`Expr<T>`, `TableExpr`, `SelectItems<...>`, `OrderSpec`), the new `List<T>` type, and meta-only types like `ModelRef` / `ColumnRef` (introduced by reflection). Its values exist only during compilation.

The two worlds intersect at **splice points**: places in the SQL grammar where a meta-world value materialises into Data-World syntax. Splice points exist today (every `smelt.<path>(...)` call site is one); the proposal extends the set so list-typed meta values can also splice.

### 3.2 Alternative framings considered

- **A. Meta-World as a separate language.** Like Liquid or Jinja — a distinct grammar, a distinct evaluator, a distinct surface. Rejected: violates constraints 2 and 5; the LSP needs one parser and one type-check pass.
- **B. Meta-World as macros.** Lisp-style `defmacro` returning AST. Rejected: macros operate on raw syntax, not typed values; the analysability story collapses.
- **C. Meta-World as an effect system.** Tag every function with whether it's "compile-time" or "runtime"; let users define their own meta functions. Rejected as too much machinery for v1; effectively requires two function languages with mostly-identical syntax. The fragment-sort approach is the smaller commitment.
- **D. Meta-World implicit, never user-visible.** Keep adding fragment sorts forever; never expose `List<T>`. Rejected: the column-iteration / model-iteration patterns require *user-constructed* lists, which forces the meta-world to be visible.

The proposal in this paper is essentially "Option D extended just enough to let users construct and manipulate lists" — the smallest move that unlocks the missing patterns.

## 4. Building Blocks

This section walks the proposed primitives one at a time, with alternatives at each step. The reader can imagine each subsection as a candidate spec section (`Surface`, `Semantics`, `Design`).

### 4.1 The `List<T>` type

A first-class meta type representing a finite, ordered sequence of meta values. `T` may be:

- A fragment sort (`Expr<Numeric>`, `TableExpr`, `Expr<Boolean>`, …).
- A meta-only type (`ModelRef`, `ColumnRef`, `Text` *as a meta literal*, `Integer` *as a meta literal*, …).
- Another `List<U>` (nested lists).

The list length is fixed at the moment of construction. Lists are immutable — `map` and `filter` produce new lists.

**Why a new type at all.** Today the closest thing is `SelectItems<Kind, ctx>`, but that type is contextually constrained (carries a `Kind` ceiling and a context binding) and only appears in select-list positions. A user writing `union by tag` wants a `List<TableExpr>`, not a `SelectItems`. A user writing `coalesce(*numerics)` wants a `List<Expr<Numeric>>`. The general type is more useful and the special-purpose ones become aliases or coercion targets.

**Alternatives:**

- **(i) Add `List<T>` as proposed.** Single new type, parameterised. Subsumes most of what `SelectItems`/`OrderSpec` do once a reducer is chosen.
- **(ii) Add separate types per intended use.** `ExprList<T>`, `TableList`, `OrderList`, etc. Closer to the existing fragment-sort style. Loses generality (can't have `List<ColumnRef>` for reflection later without minting another).
- **(iii) Reuse `SelectItems<...>` and generalise it.** Make `SelectItems<T>` accept any `T`, not just expression sorts; rename to something less SELECT-flavoured. Risks confusing users — the name says "select" but the type is generic.
- **(iv) Make `Array(T)` do double duty.** Treat compile-time and runtime arrays as the same type, distinguished only by where they're evaluated. Rejected: the runtime array is bounded by SQL semantics (homogeneous element type, runtime element values), the meta list isn't (`List<TableExpr>` has no runtime equivalent).

The paper's lean is (i). It is the smallest type-theoretic addition that handles every use case identified in §1, and it composes cleanly with the existing fragment-sort hierarchy because `T` ranges over fragment sorts and meta types alike.

**Subtyping.** Should `List<T>` be covariant in T? If `Expr<Integer> <: Expr<Numeric>`, is `List<Expr<Integer>> <: List<Expr<Numeric>>`? Lean: yes (covariant), because lists are immutable in this language. Alternative: invariant (forces explicit map). Covariant is what users expect from immutable containers in mainstream typed languages and matches the existing `SelectItems` kind subtyping.

**Empty list.** The type of `[]` is ambiguous in isolation. Bidirectional checking from the surrounding context provides the element type; an empty `[]` in an unconstrained position is a `CannotInferType` diagnostic. Alternative: introduce a `List<Bottom>` / `List<Nothing>` type and coerce upward; rejected as more theory for no user benefit.

### 4.2 List literal syntax

Proposed syntax: `[a, b, c]`, with trailing commas allowed.

The literal produces `List<T>` where `T` is the LUB of the element types. In an `Array(T)` target context (e.g. inside an `Expr<Array<U>>` position), the same surface tokens parse as a runtime array literal. Disambiguation is bidirectional, by target sort, at type-check time — the parser produces a single `LIST_LITERAL` CST node and the type checker assigns it either meta-list or data-array meaning.

**Worked example:**

```sql
-- Meta-list (caller passes a compile-time list of expressions)
SELECT smelt.functions.sum_of([revenue, tax, shipping]) AS total
FROM orders

-- Runtime array (single column whose value is a SQL array)
SELECT [1, 2, 3] AS arr
FROM dual
```

Same `[a, b, c]` tokens; different sort because the surrounding position has different expectations. This is the *Syntactic Lifting* idea from the user's framing — the same surface lifts to either world depending on context.

**Alternatives for the bracket syntax:**

- **(i) `[a, b, c]` with bidirectional disambiguation.** As proposed. Pro: minimal surface, matches existing DuckDB/Python-flavoured arrays. Con: non-local — the meaning of a literal depends on its surrounding context, which complicates LSP "show me what this is" hover during partial editing.
- **(ii) Distinct sigil for meta-lists.** `${ a, b, c }`, `meta[a, b, c]`, `' [a, b, c]` (Lispy quote), or `(| a, b, c |)`. Pro: locally unambiguous. Con: ugly, two ways to write similar things.
- **(iii) Function-style constructor.** `list(a, b, c)`. Pro: locally unambiguous, no parser ambiguity. Con: variadic; reads worse than `[...]` for long lists; also the bare name `list` is a likely user identifier.
- **(iv) `Array[a, b, c]` for runtime, `[a, b, c]` for meta.** Force the runtime case to be explicit, default the bracket to meta. Pro: meta is the new common case. Con: backwards-incompatible if `[1, 2, 3]` is already used as runtime SQL by users.

Lean: (i). The non-local meaning is real, but bidirectional checking is already pervasive in smelt for `Numeric` widening and `Concrete(T)` resolution; the cost is low. The hover-during-editing problem is a real LSP concern but solvable: hover can show "literal accepted in two contexts; current context expects `List<...>` / `Array(...)`".

**Empty literal.** `[]` is allowed, type inferred from context.

**Singleton.** `[x]` is allowed; trailing comma optional.

**Heterogeneous literals.** `[1, 'hello']` should error in both meta and runtime contexts, by the strict-by-default doctrine in `types.md`. The element types must unify under the LUB rules.

### 4.3 Spread operator `...`

Splices a `List<T>` into a position that accepts a comma-separated `T`. The operator is meta-only (length must be known at compile time).

Examples:

```sql
-- Spread into SELECT list (List<Expr<Any>> → SelectItems)
SELECT id, ...metric_exprs, created_at FROM users

-- Spread into function arguments (List<TableExpr> → variadic args)
smelt.functions.union_all(...audit_models)

-- Spread into ORDER BY (List<OrderItem> → OrderSpec)
SELECT * FROM orders ORDER BY ...sort_keys
```

**Where spread is allowed.** Anywhere comma-separated lists appear in SQL or smelt syntax: SELECT lists, GROUP BY, ORDER BY, function arguments, IN-lists, VALUES rows, the body of a `union_all` reducer, and the body of any user function whose parameter is a variadic.

**Where spread is forbidden.** WHERE clauses (no comma-list grammar), FROM clauses without an explicit reducer (a list of tables doesn't have a default join semantics), boolean composition (`x AND ...preds` requires the `and_all` reducer instead).

**Empty-list semantics.** A spread of `[]` elides itself and adjacent commas (consistent with current `SelectItems` empty-default behaviour, see `functions.md` §9). `SELECT id, ...[], created_at` becomes `SELECT id, created_at`.

**Alternatives for the spread surface:**

- **(i) `...xs` prefix operator.** As proposed. Pro: matches JS/TS, Python `*args`. Con: Three-dot tokens appear in some SQL dialects (e.g. `INTERVAL ... DAY` patterns); confirm no parse conflict.
- **(ii) `*xs` prefix.** Python-style. Pro: familiar to Python users. Con: `*` is heavily SQL-loaded (`SELECT *`, multiplication); ambiguity-prone.
- **(iii) Explicit reducer call always.** No `...`; users always write `comma_sep(xs)`, `union_all(xs)`, `and_all(xs)`. Pro: every reduction is explicit; teaches users the reducer concept. Con: verbose; common case (just splat) reads worst.
- **(iv) No spread, only HOF results return ready-made fragments.** A `map` returns `SelectItems` directly when the surrounding context expects `SelectItems`. Pro: no new operator. Con: pushes context-sensitivity into HOF return types; harder to type-check.

Lean: (i). The spread operator earns its keep by making the common case (splat) trivial while keeping reducers (§4.7) for the non-trivial cases. The tokenisation is solvable — `...` is currently unused in smelt's grammar, and a one-token lookahead distinguishes it from a possibly-malformed identifier.

### 4.4 Higher-order functions: map, filter, reduce

Three core HOFs over `List<T>`:

| HOF | Signature | Example |
|---|---|---|
| `map` | `(List<T>, T -> U) -> List<U>` | `map(cols, c => CAST(c AS Text))` |
| `filter` | `(List<T>, T -> Boolean) -> List<T>` | `filter(cols, c => c.type == 'Numeric')` |
| `reduce` | `(List<T>, Reducer<T>) -> T'` | `reduce(preds, and_all)` |

`reduce`'s output type depends on the reducer (see §4.7). For most contextual reducers, the output is a single fragment of an appropriate sort.

**Where HOFs evaluate.** At type-check time. The compiler walks the list, applies the lambda or reducer, and produces the resulting list or fragment. The result is a compile-time value; no SQL is generated until splice. This is the same machinery `smelt.define` expansion uses (`expansion.md` § "Two senses of expansion"), generalised to handle list-shaped intermediate values.

**Termination.** Lists are finite; HOFs are not recursive in the user's sense. `map`/`filter` walk once; `reduce` walks once. No fixed-point.

**Optional: `flat_map`, `zip`, `take`, `index_of`, `length`.** Not strictly necessary for the named scenarios. Add as user demand surfaces. Lean: ship `map`/`filter`/`reduce` only in v1.

**Naming alternatives:**

- **(i) `map` / `filter` / `reduce`.** As proposed. Pro: every functional language uses these names. Con: `map` collides with the SQL `Map(K, V)` `DataType` in the user's mental model.
- **(ii) `for_each` / `where` / `fold`.** SQL-flavoured. Pro: less collision with `Map`. Con: `for_each` implies imperative side-effects which we don't have.
- **(iii) Method-style (`xs.map(...)`).** Like Scala or Rust iterators. Pro: composes well with pipe (or removes the need for pipe). Con: requires a method-resolution mechanism in the type system; smelt is currently free-function-only.

Lean: (i), but flag the `Map(K, V)` collision as a documentation issue. Function names live in a different namespace from type names so there is no parser conflict.

### 4.5 Lambdas

Lambdas are needed for `map` and `filter` (the function argument). They are *not* needed for `reduce` if reducers are a closed registry (§4.7).

The fundamental question is the surface syntax, because the obvious choice (`x => body`) collides with smelt's existing named-argument syntax (`param => value`).

**Worked collision example:**

```sql
map(cols, c => CAST(c AS Text))
--        ^^^^^^^^^^^^^^^^^^^^^
-- Is `c` a named argument with value `=> CAST(c AS Text)`? (No, but the parser has to decide.)
```

**Disambiguation alternatives:**

- **(i) Position-based.** A `name => expr` token sequence is a lambda when it appears in *positional* (un-named) argument position; a named argument when it appears at the top level of an argument list. Pro: no new tokens, no syntactic ceremony. Con: the rule depends on argument position, which is not a property of the local CST — type-checker territory, not parser. Workable but careful.
- **(ii) Different token: `\x . body`.** Haskell/Coq-style. Pro: zero collision. Con: ASCII soup; users will hate it.
- **(iii) Different token: `|x| body`.** Rust-style. Pro: zero collision. Con: vertical bars are widely used in dialect SQL (string concatenation `||`, set operations); two of them in a row could already mean something.
- **(iv) Different token: `fn x => body`.** Keyword-prefixed. Pro: zero collision; consistent with smelt's `smelt.define` keyword vibe. Con: verbose; `fn x => body` is more typing than the JS-style.
- **(v) Implicit `_` parameter.** No explicit lambda; the body is an expression with `_` (or `it`) bound to the current element. `map(cols, CAST(_ AS Text))`. Pro: very terse. Con: only works for single-arg lambdas; can't bind multiple parameters; `_` may collide with a column.
- **(vi) Reuse `=>` but require parens around the lambda.** `map(cols, (c => CAST(c AS Text)))`. Pro: parser sees a parenthesised expression and disambiguates locally. Con: the parentheses are visual noise; users will forget them.
- **(vii) New keyword: `for c in cols select CAST(c AS Text)`.** SQL-flavoured comprehension syntax instead of HOFs. Pro: reads like SQL. Con: a new construct that needs its own type-checking; doesn't extend to filter/reduce uniformly.

Lean: **(iv)** — keyword-prefixed `fn x => body`. The `fn` keyword is a small ceremony that makes the lambda boundary unambiguous everywhere it appears, including inside named-argument positions for hypothetical future use. The keyword cost is one identifier (low), and `fn` reads as "function" without inviting confusion with `smelt.define` (which produces a named, top-level declaration; `fn` produces an anonymous, value-level closure).

Backup if `fn` proves too verbose: **(i)**, position-based, with a parser hook that checks "is this argument position positional?" before treating `=>` as named-arg. Implementation cost is moderate; the disambiguation cannot happen in the parser alone.

**Lambda type inference.** A lambda's parameter type is inferred from the HOF's signature: in `map(xs: List<T>, f: T -> U)`, the `f` argument's parameter is bound to type `T`. The body is type-checked under that binding. The result type `U` is inferred from the body. This is unidirectional inference (HOF → lambda), the simplest case of bidirectional checking.

**Multi-arg lambdas.** Almost always `fn (x, y) => body`. Currently no use case in §1 needs more than one argument, but the syntax should accommodate. Lean: ship single-arg only in v1; multi-arg is non-breaking later.

**Captures.** A lambda body can reference: parameters of the enclosing `smelt.define` body; meta-only names from outer scope. It *cannot* reference SQL columns from outer FROM-scope (those don't exist at meta-evaluation time). Capture is by-value (lists are immutable; this is a non-issue).

### 4.6 The pipe operator `|>`

A binary operator that turns `x |> f(args)` into `f(x, args)` (first-arg pipe).

```sql
-- Without pipe
reduce(filter(map(cols, fn c => c.name), fn n => n != 'id'), comma_sep)

-- With pipe
cols
  |> map(fn c => c.name)
  |> filter(fn n => n != 'id')
  |> reduce(comma_sep)
```

**Pipe semantics alternatives:**

- **(i) First-arg.** `x |> f(args)` ≡ `f(x, args)`. Pro: matches Google Pipe SQL, DuckDB Pipe, OCaml. Pro: HOFs naturally take their data first. Con: F# uses last-arg, Elixir uses first-arg — not a universal convention.
- **(ii) Last-arg.** `x |> f(args)` ≡ `f(args, x)`. Pro: matches F#, Haskell. Con: HOFs would have to take data last, which is awkward.
- **(iii) Placeholder.** `x |> f(_, args)` — the `_` marks where `x` goes. Pro: explicit, supports any position. Con: every call site adds noise; common case (first-arg) loses its terseness.
- **(iv) No pipe, method chains instead.** `cols.map(...).filter(...).reduce(...)`. Pro: pipe-free, method-style. Con: requires method-call syntax in the type system, which is a much bigger language change.

Lean: (i). Match the SQL ecosystem (Google/DuckDB).

**Pipe scope alternatives:**

- **(a) Meta-world only.** `|>` works for `List<T>` chains and any compile-time function call. Does not work for SQL queries.
- **(b) Meta-world + Data-world.** `|>` also works for SQL pipe-style queries (`FROM t |> WHERE p |> SELECT cols`). Adopts the Google/DuckDB pipe-SQL extension wholesale.
- **(c) Data-world only.** Skip the meta-world pipe; only support pipe-SQL.

(a) is the v1 lean. (b) is attractive but expands the proposal scope dramatically — it requires extending the SQL grammar and the planner. Worth a separate paper. (c) doesn't help with the §1 patterns.

**Worked example combining 4.4–4.6:**

```sql
-- "List of model paths, all matching tag 'audit', filtered to ones with column 'created_at',
--  unioned together"
audit_models
  |> filter(fn m => m.has_column('created_at'))
  |> map(fn m => SELECT * FROM smelt.<m.path>)
  |> reduce(union_all)
```

Notice this example assumes reflection (the `audit_models` source) and a `has_column` accessor — both deferred to §4.8.

### 4.7 Contextual reducers

Reducers fold a `List<T>` into a single fragment of an appropriate sort. They are *contextual* in the sense that the output sort depends on the input element sort and the reducer's declared rule.

Proposed v1 registry:

| Reducer | Input | Output | Identity (empty list) |
|---|---|---|---|
| `comma_sep` | `List<Expr<T>>` | `SelectItems<...>` | empty SelectItems (elides commas) |
| `and_all` | `List<Expr<Boolean>>` | `Expr<Boolean>` | `TRUE` |
| `or_any` | `List<Expr<Boolean>>` | `Expr<Boolean>` | `FALSE` |
| `union_all` | `List<TableExpr>` | `TableExpr` | error (no identity) |
| `intersect_all` | `List<TableExpr>` | `TableExpr` | error |
| `plus_chain` | `List<Expr<Numeric>>` | `Expr<Numeric>` | `0` (with type-appropriate cast) |
| `concat` | `List<Expr<Text>>` | `Expr<Text>` | empty string |

Each reducer is a value that can be passed to `reduce`. Reducers are not functions in the user-callable sense — they are a closed registry of compiler-known operations.

**Why a closed registry.** A user-defined reducer would need: (a) an associative binary operation, (b) an identity element, (c) the type system tracking that it is associative. (a) and (b) cannot be checked statically; (c) requires algebraic-effect-system machinery the language does not have. A closed registry sidesteps this: each reducer is a built-in with hard-coded, vetted semantics.

**Alternatives:**

- **(i) Closed registry as proposed.** Pro: simple, sound, predictable. Con: extension requires a compiler change.
- **(ii) User-defined reducers via `smelt.define` + a marker.** A function annotated `reducer: associative` that the compiler trusts to fold. Pro: extensible. Con: trust without verification; bad reducers produce subtly wrong SQL; the compiler cannot catch it.
- **(iii) No reducers — only spread.** Force every reduction to be expressed via spread + an explicit SQL operator (`expr1 AND expr2 AND expr3` → spread inside an AND chain). Pro: minimum new concepts. Con: many reductions don't have a natural variadic SQL surface (`UNION ALL` between two queries is clean; between *N* queries spread inside `(...) UNION ALL (...)` is awkward to express without a reducer).
- **(iv) Reducers as type-class instances.** Per-type instances of `Monoid<T>`. Pro: theoretically clean. Con: introduces type classes, which is a much bigger language change than smelt is currently committed to.

Lean: (i) for v1, with (ii) as a deferred extension once concrete pain emerges and a verification approach exists.

**Empty-list behaviour.** Each reducer declares its identity (or an error if none exists). `union_all([])` is an error because `UNION ALL` has no identity in SQL. Authors who want a safe default can write `if-empty(xs, default_table, fn xs' => xs' |> reduce(union_all))` once `if-empty` exists. (Out of v1 scope; cite as known divergence.)

**Reducer and pipe interaction.** `xs |> reduce(union_all)` should work. The pipe operator passes the list as the first argument; `reduce`'s signature is `(List<T>, Reducer<T>) -> ...`.

### 4.8 Reflection (deferred but designed-for)

Reflection is the workspace-introspection API: `smelt.models.with_tag(...)`, `smelt.columns_of(t: TableExpr)`, `smelt.sources.with_database(...)`, etc. It is the **input** to most of the §1 patterns; without it, the §4 building blocks are ergonomic improvements but don't unlock new dynamism.

The paper proposes to defer reflection to a follow-up phase but commit to its **type shape** now, so that the meta-language type system accommodates it without breaking changes.

**Meta-only types reflection introduces:**

| Type | Conceptually | Use |
|---|---|---|
| `ModelRef` | A reference to a model in the workspace | `union_all` over `List<ModelRef>` (after coercion to `TableExpr`) |
| `ColumnRef` | A reference to a single column of a model/source | `map` over `List<ColumnRef>` (per-column projection) |
| `SourceRef` | A reference to a source-table | Source enumeration |
| `Schema` | A column list with types | `columns_of(t)` returns `Schema` |

These are first-class meta values: they have types, they fit in `List<T>`, they have field accessors (`m.name`, `c.type`, `c.is_numeric`).

**Reflection API surface candidates** (sketches, not committed):

```
smelt.models                     -> List<ModelRef>
smelt.models.with_tag(t: Text)   -> List<ModelRef>
smelt.sources                    -> List<SourceRef>
smelt.columns_of(t: TableExpr)   -> List<ColumnRef>
smelt.config.var(name: Text)     -> Text  -- compile-time variable
smelt.config.env(name: Text)     -> Text  -- env var (gated)
m.path                           -> Text   -- on ModelRef
m.tags                           -> List<Text>
m.has_column(name: Text)         -> Boolean
c.name                           -> Text   -- on ColumnRef
c.type                           -> DataType (as a meta value)
c.is_numeric                     -> Boolean
```

**Alternatives for the reflection surface:**

- **(i) Functional accessors as proposed.** `smelt.models.with_tag(t)`. Pro: composes with HOFs trivially. Con: namespace pollution under `smelt.*`.
- **(ii) Object-style.** `smelt.workspace.models.where(t => 'audit' in t.tags)`. Pro: more uniform. Con: introduces method calls, a bigger language change.
- **(iii) Query-style.** `SELECT * FROM smelt.workspace.models WHERE 'audit' = ANY(tags)`. Pro: SQL-feel; uses existing query machinery. Con: confuses meta and data worlds; `smelt.workspace.models` is not a real database table.
- **(iv) Special "compile-time `WITH` block".** A new top-level construct that gathers reflection data: `meta { audit_models = smelt.models.with_tag('audit') }`. Pro: separates meta from data. Con: new syntax, fights the "meta is just types" design from §3.

Lean: (i), with the understanding that the namespace under `smelt.*` will grow and may need its own organisation.

**Determinism and caching.** Reflection results must be deterministic per workspace state (so type checking is reproducible). Reflection results must be Salsa-cached (so LSP responsiveness survives large workspaces). Both fall out of treating reflection as Salsa queries against the workspace input.

**Why reflection is deferred.** The list/HOF/lambda/pipe/spread/reducer surface stands on its own — it is useful for any user-constructed list. Reflection adds a *source* of lists from project metadata, which requires designing the metadata APIs (which keys, which paths, which Salsa queries) and is a substantial spec on its own. Sequencing concerns aside, the §4.1–§4.7 surface is what the meta-language *is*; reflection is what it *queries*.

### 4.9 Meta variables / let-bindings

The user's earlier framing mentions `@variables`. Are local meta bindings needed?

```sql
-- Hypothetical
@let dims = ['region', 'category', 'channel']
SELECT
  ...map(dims, fn d => SUM(amount) FILTER(WHERE dim = d) AS d)
FROM events
```

**Alternatives:**

- **(i) No new construct — use function parameters.** Hoist the binding into a helper function: `smelt.define dim_pivot(dims: List<Text>) AS (...)`. Pro: zero new syntax. Con: forces function definitions for one-off bindings.
- **(ii) Add `@let` (or `let ... in`).** Named meta-binding scoped to the surrounding declaration. Pro: ergonomic. Con: another construct to teach; needs scoping rules.
- **(iii) Add `WITH META` block.** Reuse SQL's `WITH` syntax with a meta marker. Pro: familiar shape. Con: now there are two `WITH` flavours.

Lean: (i). Function parameters cover every case so far identified, and "extract a helper" is a reasonable diagnostic when a long inline meta-binding is wanted. (ii) is non-breaking to add later if a pattern emerges where it would be much cleaner.

## 5. Worked Examples

This section composes §4 building blocks against the §1 catalogue. Each example shows the full surface; nothing is hand-waved.

### 5.1 Sum of arbitrary numeric columns

```sql
-- functions/aggregates/sum_of.sql
smelt.define sum_of(cols: List<Expr<Numeric>>) -> Expr<Numeric> AS (
    cols |> reduce(plus_chain)
)
```

```sql
-- Caller
SELECT smelt.functions.aggregates.sum_of([revenue, tax, shipping]) AS total
FROM orders
```

Compile-time expansion (after meta-evaluation, before SQL emission):

```sql
SELECT (revenue + tax + shipping) AS total
FROM orders
```

### 5.2 Coalesce all numeric columns of a model (requires reflection)

```sql
-- functions/numeric_safe.sql
smelt.define coalesce_numeric(t: TableExpr) -> SelectItems<Scalar, t> AS (
    smelt.columns_of(t)
      |> filter(fn c => c.is_numeric)
      |> map(fn c => COALESCE(c.name, 0) AS c.name)
)
```

```sql
-- Caller
SELECT id, ...smelt.functions.coalesce_numeric(orders)
FROM orders
```

The `c.name` interpolation in the lambda body is a meta-to-data lift: a compile-time string becomes a column identifier in the produced SQL. This is an example of where the type system has to handle a value crossing layers (`Text` in meta, identifier in data). Worth its own careful spec when reflection ships.

### 5.3 Union all models matching a tag (requires reflection)

```sql
-- analytics/all_audit_events.sql
SELECT * FROM (
  smelt.models.with_tag('audit')
    |> map(fn m => SELECT * FROM smelt.<m.path>)
    |> reduce(union_all)
)
```

The `smelt.<m.path>` syntax is borrowed from the existing addressing scheme (`architecture.md` §"Resolution"). At meta-evaluation time, `m.path` is a compile-time string that becomes a `smelt.<path>` reference resolved like any other.

### 5.4 Per-environment dimension list

```sql
-- analytics/dim_pivot.sql
SELECT
  date,
  ...map(smelt.config.var('pivot_dims'),
         fn d => SUM(CASE WHEN dim = d THEN amount END) AS d)
FROM events
GROUP BY date
```

`smelt.config.var(...)` returns a `List<Text>` from project config. The lambda produces an `Expr<Numeric>` per element with `AS d` aliasing — same meta-to-identifier lift as §5.2.

### 5.5 Surrogate key over column list

```sql
smelt.define surrogate_key(cols: List<Expr<Text>>) -> Expr<Text> AS (
  cols |> reduce(concat_with('|')) |> md5
)
```

`concat_with('|')` is a parameterised reducer — sketchy in v1 (closed registry doesn't accommodate parameters). A v1 workaround: `reduce(concat)` after intercalating the separator. Or expand the reducer registry to include `concat_with(sep: Text)`. Either is a small extension.

### 5.6 Generic test parameterised over columns

```sql
---
materialization: test
---
WITH bad AS (
  SELECT * FROM smelt.<model>
  WHERE NOT (
    smelt.config.var('check_cols')
      |> map(fn c => c IS NOT NULL)
      |> reduce(and_all)
  )
)
SELECT 'rows with nulls in required columns' AS reason FROM bad
```

The test asserts that every column in the configured `check_cols` list is non-null. The same test body works for any model and any column set without macro-style duplication.

## 6. Static Analysis & LSP Implications

This is where the proposal earns its keep over Jinja. Each §4 primitive is designed so the LSP can give the user the same ergonomic feedback they get for static SQL.

### 6.1 Goto-definition through HOFs

When a user clicks on `c.name` inside `map(cols, fn c => ... c.name ...)`, the LSP must resolve to the column declaration in the source `cols`. Mechanism: meta-evaluation tracks each list element back to its source span (the same `Caller(span)` / `Callee(fn_id, span)` provenance machinery `expansion.md` already specifies). The lambda body's `c` binding inherits the per-element span; `c.name` resolves through the field accessor to the original column.

### 6.2 Hover

Every meta value has a type. Hovering over `audit_models` in §5.3 shows `List<ModelRef>`. Hovering over `c` inside the lambda shows `ColumnRef`. Hovering over the result of `reduce(union_all)` shows `TableExpr`. None of this requires running anything at the database — all type information is static.

### 6.3 Diagnostic frame stacks

The `expansion.md` frame-stack contract extends to meta-evaluation. A type error inside a lambda body, reached through `map(xs, ...)`, surfaces with a frame stack:

```
in expansion of map: lambda parameter `c` was bound to `ColumnRef`
  at functions/numeric_safe.sql:5:23
in expansion of coalesce_numeric: parameter `t` was bound to `orders`
  at models/orders_safe.sql:2:38
```

Multi-level frames work because each HOF call pushes a frame the same way `smelt.<path>(...)` calls do.

### 6.4 Schema inference for dynamic columns

The §5.2 `coalesce_numeric` produces a `SelectItems<Scalar>` whose contents depend on `t`'s schema. If the LSP can resolve `t`'s schema (via `smelt.columns_of(t)`), it can statically infer the resulting select-list shape. The output schema of a model using `coalesce_numeric` is therefore *known at compile time*, even though the column list is dynamic.

This is the property dbt fundamentally cannot provide for macro-generated columns: dbt's schema inference falls back to "run the query and see" for any dynamism. smelt's meta-evaluator can compute the schema directly.

### 6.5 Refactoring (rename)

A column rename in a source table propagates through reflection-driven meta evaluation: every `c.name` in a lambda body that fed from `smelt.columns_of(source)` updates. The LSP sees this because the meta-evaluator sees this. dbt can only do textual rename within macro bodies (lossy and unsound).

### 6.6 Performance

Meta-evaluation is performed during `file_diagnostics` and is Salsa-memoised. Workspace-wide reflection queries (`smelt.models.with_tag(...)`) are themselves Salsa queries against the workspace input, automatically invalidated when relevant files change. No re-evaluation cost beyond the smelt-db caching that already exists.

## 7. Comparison with Prior Art

### 7.1 dbt + Jinja

The benchmark. Strengths: ubiquity, every dbt user knows Jinja. Weaknesses: untyped, opaque to LSP, errors anchor to post-substitution SQL (hard to read), no static schema inference for macro-generated columns. The proposal in this paper produces a strict superset of dbt's dynamism with none of these weaknesses.

### 7.2 SQLMesh

SQLMesh uses a Jinja variant but has a richer "macros as Python functions" path. Macros can return Python values (lists, dicts) that flow into SQL. Strengths: Python expressivity; weaknesses: same as Jinja for analysability — the Python evaluator is not a type checker, the LSP cannot follow macro return values into SQL.

### 7.3 Malloy

Malloy avoids the templating problem by being a different surface language. Its `dimension` and `measure` declarations are typed. Its semantic model is closer to Cube/LookML. Strengths: clean by construction. Weaknesses: full migration; not SQL.

The proposal in this paper aims for Malloy-like typing on a SQL-native surface — a different trade-off than Malloy made.

### 7.4 PRQL

PRQL is a different surface language (pipeline-style) that compiles to SQL. The pipe operator is the primary construct. PRQL is helpful as evidence that pipe-style composition reads well for SQL transformations, supporting §4.6.

### 7.5 dbt-utils

dbt-utils is the *de facto* standard library of dbt macros. The patterns it implements (`get_column_values`, `pivot`, `union_relations`, `surrogate_key`) are exactly the §1 catalogue. They serve as the user-facing benchmark: a smelt user with the proposal in this paper should be able to reimplement dbt-utils in user space, with full LSP support.

### 7.6 Terra and MetaML

Terra (Devito et al.) and MetaML (Taha & Sheard) are the academic precedent for typed staged code generation. The proposal here is closer to Terra's "AST template parameters with type tags" than MetaML's full multi-stage programming. Smelt has a single expansion phase; Terra and MetaML support nested stages. The simpler model is appropriate for smelt's scope.

## 8. Open Questions

**Reducer extensibility.** §4.7 leans closed-registry. If users want `concat_with(sep)`-style parameterised reducers, the registry needs a parameterised-reducer variant. Where is the line between "built-in reducer with a parameter" and "user-defined reducer"?

**Lambda type annotations.** Should `fn x => body` allow `fn (x: T) => body`? Currently inferred from the HOF signature; explicit annotation is non-breaking to add.

**Multi-list HOFs.** `zip` produces `List<(A, B)>` but smelt has no tuple type. Adding tuples is a substantial extension. Alternative: `zip_with(xs, ys, fn (x, y) => z)` returning `List<Z>` — no first-class tuples needed. Worth exploring.

**Order of evaluation.** `[a, b, c]` then `map(fn x => f(x))` — does `f(a)` happen before or after `f(b)`? At meta-evaluation time the answer matters only if reflection has side effects (which it must not). Lean: declare meta-evaluation pure; document that any non-pure helper is a bug.

**Pipe-SQL extension.** §4.6 alternative (b) ports the pipe operator into SQL itself. Worth a separate paper. Does not block the meta-language.

**Meta-let / `@variables`.** §4.9 lean is "no new construct". If users complain, revisit.

**`Map<K, V>` meta type.** Not proposed. The §1 patterns are list-shaped. If a key-value structure becomes necessary (e.g. "for each model, its column count"), `List<(ModelRef, Integer)>` works once tuples exist. Defer.

**Heterogeneous lists.** The proposal assumes monomorphic `List<T>`. A user wanting a list of mixed types must use a sum type (which smelt doesn't have) or accept LUB widening. Not a blocker for the §1 patterns.

**Identifier-vs-string in lambda bodies.** §5.2 uses `c.name` as both a string (meta) and an identifier (data) inside the same expression. The crossing rule needs careful spec when reflection ships. Candidate rule: a meta `Text` value placed where the SQL grammar expects an identifier compiles to that identifier; outside identifier positions it remains a string literal. Needs adversarial test cases.

**Backward compatibility.** Existing `smelt.define` functions taking `SelectItems<Kind, ctx>` parameters keep working. The new `List<Expr<...>>` is purely additive. If §4.1 alternative (iii) is chosen (unify `SelectItems` into `List`), it becomes a breaking change. Lean (i) avoids this.

**Spread in FROM clause.** `FROM ...models` has no obvious join semantics. Currently reject. If users want `CROSS JOIN UNNEST`-style, that's a different request (operates on runtime arrays, not meta lists).

**Recursive reflection.** Can a meta function call `smelt.columns_of(t)` where `t` is the model currently being type-checked? Probably yes (the model's schema is a fixed point of its body), but workspace cycles need handling. Defer to reflection spec.

## 9. Sequencing — A Sketch

The full proposal is large. The user has already asked for "literal lists first" as a way to test the lifting/compile-time mechanism without the rest. A plausible sequencing (subject to revision):

1. **Phase A — Lifting test.** `List<T>` type, list literal syntax `[a, b, c]`, spread `...`. No HOFs, no lambdas, no reflection. Validates the meta/data world boundary.
2. **Phase B — HOFs, no reflection.** Add `map`/`filter`/`reduce`, lambdas, contextual reducers. Users can compose user-supplied lists. Pipe operator lands here.
3. **Phase C — Reflection, narrow.** `smelt.columns_of(t)`, `smelt.config.var(...)`. The smallest reflection surface that unlocks several §1 patterns.
4. **Phase D — Reflection, wide.** `smelt.models.*`, `smelt.sources.*`, `ModelRef` / `SourceRef`. Full project introspection.
5. **Phase E — Polish.** Pipe-SQL extension if compelling, parameterised reducers, multi-arg lambdas, meta-let if needed.

Each phase is committable in isolation, each is non-breaking for the next, each unlocks a meaningful subset of §1.

This sequencing is illustrative, not prescriptive. The point of the paper is the building blocks and their alternatives, not the order. The shipping order is a separate plan.

## 10. Recommendation

Build the meta-language. The §4 building blocks have small, locally-justifiable additions to existing machinery: `List<T>` extends the type system by one type, the literal extends the parser by one production, spread/HOFs/lambdas/pipe/reducers each touch the type checker but not the planner. Reflection is the substantial work and is rightly deferred.

The dbt parity claim is achievable. The dbt-exceedance claim — full LSP, type-checked HOFs, schema inference for dynamic columns, frame-stack diagnostics into meta — is what justifies the work over staying in `smelt.define`-only territory.

The two highest-leverage open decisions are:

- **Lambda syntax** (§4.5). Affects every HOF call site. The `=>` clash with named arguments has no costless solution. Lean: keyword-prefixed `fn x => body`, with positional disambiguation as backup.
- **Reflection API shape** (§4.8). Affects every §1 pattern that depends on workspace introspection. Lean: functional accessors under `smelt.*`, deferred to a follow-up spec, but type shape committed now.

Everything else in §4 has a low-regret default that matches the existing language style.

## 11. References

- `docs/specs/functions.md` — `smelt.define`, fragment sorts, frontmatter
- `docs/specs/types.md` — `DataType` vocabulary, fragment-sort semantics
- `docs/specs/expansion.md` — codegen-time expansion, frame stacks, provenance
- `docs/specs/scoping.md` — body scoping, splice contexts, parameters-first
- `docs/specs/architecture.md` — `smelt.<path>` resolution, project layout
- `docs/research/20260413-smelt-functions.md` — fragment sorts and the case for typed SQL composition
- dbt-utils — https://github.com/dbt-labs/dbt-utils (the §1 / §7.5 benchmark)
- DuckDB Pipe Syntax — https://duckdb.org/docs/sql/dialect/pipe_syntax
- Google SQL Pipe — https://cloud.google.com/blog/products/data-analytics/the-power-of-pipes-in-sql
- PRQL — https://prql-lang.org/ (pipeline-style SQL, evidence for §4.6)
- Malloy — https://www.malloydata.dev/ (typed-DSL precedent, §7.3)
- Terra — Devito et al., "Terra: a Multi-Stage Language for High-Performance Computing"
- MetaML — Taha & Sheard, "MetaML and Multi-Stage Programming with Explicit Annotations"
