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
- **Multi-model production mechanism** — §4.10.4 leans on a frontmatter directive + `List<ModelDef>` body. The alternative (a top-level `smelt.generate.models { ... }` construct) is non-trivial to undo if shipped, and the path-naming rule for generated models is similarly load-bearing. Decide before any meaningful Phase E work.

**Author confidence (subjective — for a reviewer's calibration):**

- **High** — §3 (meta-world recap), §6 (LSP exceedance over dbt), §8 (open-question enumeration).
- **Medium** — §4.1 (`List<T>`), §4.4 (HOF set), §4.6 (pipe), §4.7 (reducer registry), §4.10.2 (records — record types are small and well-precedented), §4.10.3 (`Map<K, V>` — the reduced API set is conservative).
- **Low** — §4.5 (lambda surface — `fn` is a guess), §4.8 (reflection API — sketch only), §4.10.1 (config loader API — typing approach is leaning but unsettled), §4.10.4 (multi-model production — the deepest workspace-shape change in the paper, several alternatives with no clearly dominant choice), §4.10.5 (meta-`Text`-as-identifier lift — implicit vs. explicit cast, narrowness rule), §5.2 / §5.5 / §5.7 / §5.8 (worked examples lean on undecided sub-questions).

**Consciously out of v1 scope** (per §1.2 and §8): hooks / lifecycle events, *general* custom materialisations as a planner-rule extension point (multi-model production in §4.10.4 is in scope as a *workspace-shape* concern, not a planner-rule one), pipe-SQL extension into the data world, meta-let / `@variables`, tuples, recursive reflection, user-extensible reducers, generators-of-generators, sum / variant types. **Pulled back into scope by §4.10:** `Map<K, V>` (the §1 patterns were list-shaped and could defer; the §4.10 use case makes `Map` the dominant entry-point type).

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
| Generate staging models from a sources YAML | `dbt-codegen` macros | External config + records + multi-model production |
| Per-tenant model variants from a tenants list | per-project copies or external codegen | Multi-model production + meta-`Text`-as-identifier |
| Metric / KPI definitions become individual models | `dbt-metrics`, SQLMesh metrics | Records + multi-model production |

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
- **Custom materialisations** as a user-extension point. smelt's planner-rule API will eventually serve this need; that is a separate spec (`planner_integration.md`). The §4.10 multi-model-production case touches the boundary — one file generating several models — but is a *workspace-shape* concern (which models exist), not a *materialisation* concern (how each model materialises). The two compose; neither subsumes the other.
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

### 4.10 Config-driven model generation

The §1 catalogue covered patterns where dynamism lives *inside* a model: the column list flexes, but the model itself is one user-authored file producing one node in the dependency graph. A second class of patterns has dynamism live *across* models — a single configuration source produces *multiple* models. dbt approximates this with `dbt-codegen` (literal codegen of `.sql` files), per-tenant projects, and `dbt-metrics` (each metric becomes an effective model). The shape: a YAML / JSON file describes a list of "things"; each thing should become a top-level model.

This is structurally different from §1. There, meta-evaluation produced a fragment that spliced into one user-written model. Here, meta-evaluation produces N model definitions where there were previously zero user-written model files for them. Several of §4.1–§4.9's building blocks contribute (HOFs, lambdas, pipe, spread), but five new pieces are required:

- **4.10.1** External configuration sources — a typed source of meta values from disk.
- **4.10.2** Record (struct) meta types — to represent each entry of the config.
- **4.10.3** `Map<K, V>` meta type (revisited from §8) — for keyed configs.
- **4.10.4** Multi-model production — one file producing N models.
- **4.10.5** Meta-`Text` as identifier (sharpened from §5.2 / §8) — to construct generated model paths.

The biggest is §4.10.4 — the only piece that is conceptually new at the workspace level. Everything in §4.1–§4.9 produced compile-time values that splice into existing user-written models. §4.10.4 produces models themselves. The remaining four pieces are type-system extensions that are locally justifiable.

#### 4.10.1 External configuration sources

Need: load a workspace-relative file at compile time and treat its contents as a typed meta value.

**API sketch.**

```
smelt.config.load_yaml(path: Text, schema: Schema)  -> Schema-typed value
smelt.config.load_json(path: Text, schema: Schema)  -> Schema-typed value
smelt.config.load_toml(path: Text, schema: Schema)  -> Schema-typed value
```

The schema declares the expected shape (§4.10.2); the loader parses the file and validates against the schema. Validation failure produces a compile-time diagnostic anchored to the offending file/line of the YAML.

**Determinism.** Files must live inside the workspace and become Salsa-tracked inputs. No HTTP/network reads, no clock, no env vars without explicit gating. A schema and a file together are a pure function from workspace state to a meta value — same property §6.6 requires for reflection.

**Per-target variants.** dbt has `target.name` and per-target `var()`. A few options:

- (i) Path interpolation in the loader: `load_yaml('configs/{target}/sources.yaml', S)` once string interpolation lands.
- (ii) Overlay: load `sources.yaml`, merge `sources.{target}.yaml` if present.
- (iii) Post-load `filter`: load all entries, filter on a per-row `target` field.

Lean: (i) and (iii) as primary. (ii) only if a concrete need surfaces.

**Alternatives for the typing approach:**

- **(a) Untyped, JSON-shaped value.** Returns a dynamic `Json` with `.field`, `.[i]`, `.as_text` operators that may fail. Pro: zero schema authoring. Con: loses static checking, autocompletion, definition-site errors — pushes failures to the use site, which is exactly the dbt failure mode this paper exists to fix.
- **(b) Schema-typed (proposed).** User declares a schema; loader validates and returns a typed value. Pro: full static typing, hover, autocompletion, failures land at the YAML line that violated the schema. Con: schema authoring required.
- **(c) Schema inferred from data.** Read the file, infer shape, hand back a typed value. Pro: zero ceremony. Con: schema becomes data-dependent — refactoring the YAML silently changes the type, breaking distant code without a definition-site signal.
- **(d) Schema inline at the call site only.** `load_yaml(path, { name: Text, columns: List<Text> })`. Pro: schema reads adjacent to use. Con: duplicated when the file is loaded twice; bad for large schemas.

Lean: **(b)**, with both standalone schema declarations (§4.10.2) and inline schemas as sugar. Inline schemas are reasonable when the schema is used once.

**File-format coverage.** YAML is dominant in this ecosystem; ship that first. JSON and TOML ride along cheaply (parsers exist). CSV is tempting (column lists in spreadsheets) but lacks a schema-friendly type system; defer.

#### 4.10.2 Record (struct) meta types

Each loaded config row is a thing with named fields. The meta-language needs a record type.

**Surface candidates:**

- **(i) `Record<{name: Text, columns: List<Text>}>`** — type-as-shape, inline.
- **(ii) Named declarations.** `smelt.record SourceEntry = { name: Text, columns: List<Text> }`, addressable as `smelt.records.SourceEntry` (or wherever the records namespace lands).
- **(iii) Reuse the data-world `Struct({...})`** with a meta marker — share the spelling.
- **(iv) Row types** `{ name :: Text, ... }` — Haskell/Elm-flavour.

Lean: **(i) and (ii) both supported**. Inline records suit one-shot schemas inside a HOF callback; named records suit shapes that recur across files and want their own goto-definition target.

**Why not reuse `Struct({...})`.** Constraint 6 (compile-time only) requires a clean meta/data separation. The user must be able to tell whether a record value persists into runtime; sharing the spelling obscures that. Same reasoning as §3.1's argument for `[a, b, c]` ↔ `Array(T)` disambiguation, applied to records.

**Field access.** `entry.name`, `entry.columns`. Already consistent with the `c.name` accessors §5.2 and §4.8 (reflection's `ColumnRef`) preview.

**Width subtyping.** A record `{a: T, b: U}` is a subtype of `{a: T}`. Lean: yes — the same conservative covariance §4.1 lands on for `List<T>`. Allows passing a richer-than-required record into a HOF expecting a narrower one.

**Equality and ordering.** Structural equality at meta-evaluation. No defined ordering; sorting requires explicit `sort_by(fn r => r.field)` (deferred — `sort_by` is not in the v1 HOF set §4.4).

#### 4.10.3 `Map<K, V>` meta type

YAML mappings naturally produce `Map<K, V>`. The §8 deferral was correct under §1's list-shaped patterns; §4.10 makes `Map` the dominant type at the entry point of many config schemas (e.g. `tenants: { acme: {...}, globex: {...} }`).

**Reduced API:**

| Op | Signature |
|---|---|
| `entries` | `Map<K, V> -> List<Record<{key: K, value: V}>>` |
| `keys` | `Map<K, V> -> List<K>` |
| `values` | `Map<K, V> -> List<V>` |
| `get` | `(Map<K, V>, K) -> V` (compile error on missing — or `Optional<V>` if optionals exist) |
| `has` | `(Map<K, V>, K) -> Boolean` |

The canonical idiom is `m |> entries |> map(fn kv => ...)` — convert to a list of records and run §4.4 HOFs over it. Map-shaped HOFs (`map_values`, `map_entries`) can wait.

**Alternatives:**

- (i) Add `Map<K, V>` with the reduced API. Pro: matches YAML naturally. Con: another type.
- (ii) Skip; require all configs to be lists-of-records. Pro: minimal change. Con: forces config authors to restructure YAML that is naturally keyed.
- (iii) `Map<K, V>` as sugar for `List<Record<{key: K, value: V}>>`. Pro: zero new type. Con: lookup is O(n); diverges from user expectation; `get('missing_key')` becomes a runtime-style failure.

Lean: **(i)**.

**Heterogeneous values.** `Map<Text, ?>` (YAML map whose values vary in shape) — same answer as heterogeneous lists in §4.2: must unify under LUB or be rejected. Real heterogeneous maps need sum types, which are out of v1 scope.

#### 4.10.4 Multi-model production

The deepest change. Today: one `.sql` file = one model = one node in the dependency graph. Proposal: one file can produce N models, each a normal node.

**Mechanism options:**

- **(i) Frontmatter directive + body returns `List<ModelDef>`.** The file declares it produces a list of models. The body is a meta-evaluable expression of type `List<ModelDef>`. The compiler synthesises N models in the workspace from the evaluated list.

  ```
  ---
  generates: models
  ---
  smelt.config.load_yaml('sources.yaml', SourceEntry)
    |> map(fn e => ModelDef {
         name: 'staging_' ++ e.name,
         body: SELECT * FROM smelt.sources.<e.source_table>,
         materialization: if e.is_incremental then 'incremental' else 'view'
       })
  ```

- **(ii) Top-level construct.** `smelt.generate.models { ... }` — a new top-level form alongside `smelt.define`. Pro: explicit syntactic boundary. Con: introduces another top-level form; the symmetry between "this file *is* a model" and "this file *produces* models" is asymmetric to learn.
- **(iii) External codegen.** Skip the language extension; users run a separate code-gen tool that writes `.sql` files. Pro: zero language change. Con: loses LSP, types, smelt's reason to exist over dbt.
- **(iv) Implicit per-row materialisation.** A model's body produces `List<Row>` and the planner materialises one model per row. Pro: no new construct. Con: the resulting models lack distinguishability in the workspace; conceptually odd; collides with the data-world `List`/`Array` interpretation.

Lean: **(i)**. Frontmatter is the existing extension point for per-file directives; reusing it is the smallest change. The convention `<name>.gen.sql` (file naming) is optional sugar — the directive is what marks the file.

**The `ModelDef` shape (rough):**

```
ModelDef = Record<{
    name: Text,                     -- final segment of the generated path
    body: TableExpr,                -- the SQL
    materialization: Text,          -- defaults to view
    description: Text,              -- optional, for docs
    tags: List<Text>,               -- for §4.8 reflection-driven filters elsewhere
    -- frontmatter-equivalent fields
}>
```

A `ModelDef` is *not* a `ModelRef` (§4.8). `ModelRef` is a workspace-introspection handle (a pointer to an existing model); `ModelDef` is a constructor (a recipe to make one). They are dual.

**Generated model paths.** Under (i), the natural paths nest under the generator file. A generator at `models/staging/sources.gen.sql` producing a row with `name = 'orders'` yields `smelt.models.staging.sources.orders`. Alternatives:

- **(a)** Path = generator-file's path + `.<name>` (most predictable; can be deep).
- **(b)** Generator file is "transparent" — generated models live at the parent directory (`smelt.models.staging.<name>`). Cleaner; collisions across generators in the same directory.
- **(c)** Frontmatter override: `generated_path_prefix: models.staging`. Full control; another knob.

Lean: **(a) by default, (c) as escape hatch**.

**LSP implications.**

- *Workspace queries.* `smelt.models.with_tag('audit')` (§4.8) must include generated models. The Salsa "list all models" query derives both authored and generated.
- *Goto-definition.* Clicking on `smelt.<staging_orders>` from another file jumps to the generator file with the originating config row contextualised — multi-frame stack: "generated by `staging_orders` `ModelDef` at `sources.gen.sql:9` from `sources.yaml:14`".
- *Hover.* Shows the generated path, the generator file, the originating config row, and the inferred output schema.
- *Rename.* Renaming the `name` field of a config row updates the generated model path. References in other models follow the path.

**Cycles.** A generator can produce models that other models (authored or generated) depend on. Standard cycle detection extends naturally — the dependency graph *after* generation is what's checked. The subtle case: generator A's body depends on the schema of a model that generator B produces (cycle in generator evaluation, not just in run-time data). Lean: **forbid initially**; require generators to have static schemas resolvable without other generators having run. Revisit if real use cases appear.

**Determinism.** Same workspace state must produce the same set of generated models with the same paths. Falls out of meta-evaluation purity (constraint 4) and Salsa-tracking of all inputs.

**Comparison with §1.2's "custom materialisations".** Custom materialisations ask "how does *this* model materialise?" (a planner-rule concern). Multi-model production asks "what are the models?" (a workspace-shape concern). The two intersect — a generator declares per-model materialisation in `ModelDef.materialization` — but neither subsumes the other. This proposal commits only to passing the materialisation choice through to existing planner-rule machinery.

#### 4.10.5 Meta-`Text` as identifier

To say `'staging_' ++ e.name` becomes a *path component* of a generated model, the type system must allow a meta `Text` to be used as an identifier in identifier positions. §5.2's `c.name AS c.name` and §5.4's pivot aliases preview this; multi-model production makes it required.

**Constraints:**

- The `Text` must match smelt's identifier grammar (no leading digits, no reserved keywords, etc.). Validation runs at meta-evaluation, on a string-pure function.
- The `Text` must be deterministic.
- Validation failure produces a diagnostic anchored to the source span where the bad `Text` was constructed (or the closest enclosing meta evaluation step), via the §6.3 frame stack.

**Surface options:**

- **(i) Implicit lift.** A meta `Text` in identifier position is treated as an identifier. Pro: terse — `'staging_' ++ e.name` just works. Con: errors land at use sites that can be far from the bad input.
- **(ii) Explicit cast.** `as_ident('staging_' ++ e.name)`. Pro: pinpoints the cast point. Con: ergonomic noise on every interpolation.
- **(iii) A distinct `Identifier` meta type.** With a partial `Identifier::of(t: Text) -> Identifier`. Pro: type system catches bad-id errors at construction site. Con: more types; users must know when to cast; diverges from §4.2's bidirectional lifting style.

Lean: **(i) implicit, with (ii) explicit available when the user wants the cast point pinned**. Implicit lift is consistent with §4.2's bracket literal lifting to either `Array(T)` or `List<T>` based on context. The frame stack handles the "errors land at use sites" weakness — `to_ident` failures surface at the construction site with a frame "lifted to identifier in `ModelDef.name` position at <path>".

**Narrowness.** The lift permits using a meta `Text` as: a model path segment, a generated column alias, a generated CTE name, an aggregate or function alias. It does *not* permit using a meta `Text` as an arbitrary SQL keyword (e.g. substituting `FROM` with a string), which is string-template territory and re-introduces the dbt failure mode this paper exists to avoid. The distinction is the SQL grammar slot the `Text` occupies — identifier slots only.

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

### 5.7 Generate staging models from a sources YAML (requires §4.10)

```yaml
# configs/sources.yaml
- name: orders
  source_table: raw.orders
  columns: [id, customer_id, amount, created_at]
  is_incremental: true
- name: customers
  source_table: raw.customers
  columns: [id, name, email, created_at]
  is_incremental: false
```

```sql
-- models/staging/sources.gen.sql
---
generates: models
---
smelt.record SourceEntry = {
    name: Text,
    source_table: Text,
    columns: List<Text>,
    is_incremental: Boolean
}

smelt.config.load_yaml('configs/sources.yaml', SourceEntry)
  |> map(fn e => ModelDef {
       name: 'staging_' ++ e.name,
       body: SELECT ...map(e.columns, fn c => c)
             FROM smelt.sources.<e.source_table>,
       materialization: if e.is_incremental then 'incremental' else 'view'
     })
```

This produces two models in the workspace: `staging_orders` (incremental) and `staging_customers` (view). Each can be referenced from other models as `smelt.models.staging.sources.staging_orders` etc., shows up in workspace reflection (§4.8), gets schema inferred (§6.4) — column lists are statically known from the YAML — and supports goto-definition back to the YAML row that produced it.

The `if-then-else` on `materialization` is a small extension not currently in §4 (lean: ship `if expr then a else b` as a meta-world ternary; §9 phase B; non-blocking for the rest).

### 5.8 Per-tenant model variants (requires §4.10)

```yaml
# configs/tenants.yaml
acme:
  schema: acme_prod
  enabled_features: [billing, audit]
globex:
  schema: globex_dev
  enabled_features: [billing]
```

```sql
-- models/per_tenant/orders.gen.sql
---
generates: models
---
smelt.record TenantConfig = {
    schema: Text,
    enabled_features: List<Text>
}

smelt.config.load_yaml('configs/tenants.yaml', Map<Text, TenantConfig>)
  |> entries
  |> filter(fn kv => 'billing' in kv.value.enabled_features)
  |> map(fn kv => ModelDef {
       name: 'orders_' ++ kv.key,
       body: SELECT * FROM smelt.<kv.value.schema>.orders,
       materialization: 'view'
     })
```

The keys-as-tenant-names pattern requires §4.10.3's `Map<K, V>`. Models `orders_acme` and `orders_globex` are produced; both have statically-known schemas inherited from their respective `<schema>.orders` source.

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

**Schema authoring overhead for config loading.** §4.10.1 lands on schema-typed loading. For one-off configs, schema declarations are a tax. Two ergonomic options worth exploring: (a) inline-only schemas at the call site for one-shot use; (b) a future `infer_schema` mode that produces a schema declaration from a sample file at the user's request (codegen, not runtime inference). Both deferred until shipping reveals the pain.

**Generator file naming.** Is `.gen.sql` (file-extension convention) load-bearing or just sugar? Lean: sugar; the `generates: models` frontmatter is what marks the file. But editor tooling and human readers benefit from the visual distinction. Recommend convention without enforcement.

**Generated model paths — depth vs. flatness.** §4.10.4 leans on nested paths under the generator file. For deep directory trees this gets verbose (`smelt.models.staging.sources.staging_orders`). The frontmatter override (option (c)) is the escape hatch but easy to misuse. Worth a usability pass once a real example workspace exists.

**Generators-of-generators.** §4.10.4 forbids cycles in generator evaluation. The non-cycle multi-stage case — generator B reads YAML produced by generator A — is currently also forbidden by the static-schema rule. This forecloses some valid use cases (e.g. one config file describing other config files). Defer; the simple flat case is far more common and can be shipped first.

**Per-target config overlay.** §4.10.1 lists path interpolation, overlay, and post-load filter. Only one needs to ship initially, but pinning down which is a v1 decision because users will lock in idioms quickly.

**Multi-model identity stability under refactoring.** If a config row's `name` field is renamed, the generated model path changes — every reference elsewhere breaks unless the LSP's rename driver follows the generator and propagates. Refactoring tooling for generators is a non-trivial item; design needs explicit attention.

**Validation diagnostics for schema mismatches.** When a YAML file fails its declared schema, the diagnostic should anchor on the YAML line and column, not on the smelt loader call. Requires the YAML parser to retain source spans through validation. Implementation cost is moderate; deferring it produces a noticeably worse error UX.

## 9. Sequencing — A Sketch

The full proposal is large. The user has already asked for "literal lists first" as a way to test the lifting/compile-time mechanism without the rest. A plausible sequencing (subject to revision):

1. **Phase A — Lifting test.** `List<T>` type, list literal syntax `[a, b, c]`, spread `...`. No HOFs, no lambdas, no reflection. Validates the meta/data world boundary.
2. **Phase B — HOFs, no reflection.** Add `map`/`filter`/`reduce`, lambdas, contextual reducers. Users can compose user-supplied lists. Pipe operator lands here.
3. **Phase C — Reflection, narrow.** `smelt.columns_of(t)`, `smelt.config.var(...)`. The smallest reflection surface that unlocks several §1 patterns.
4. **Phase D — Reflection, wide.** `smelt.models.*`, `smelt.sources.*`, `ModelRef` / `SourceRef`. Full project introspection.
5. **Phase E — Config-driven generation (§4.10).** External config loading, records, `Map<K, V>`, multi-model production, meta-`Text`-as-identifier. Largest individual phase by surface area. Depends on Phase B (HOFs) and overlaps Phase C (`smelt.config.var(...)` is the simplest config primitive). Multi-model production is the substantive item; the type-system additions (records, `Map`) are small in isolation. Plausible to split into E1 (config + records + `Map`, no multi-model) and E2 (multi-model production) if E proves too large in one bite.
6. **Phase F — Polish.** Pipe-SQL extension if compelling, parameterised reducers, multi-arg lambdas, meta-let if needed, `if-then-else` ternary for the meta-world.

Each phase is committable in isolation, each is non-breaking for the next, each unlocks a meaningful subset of §1.

This sequencing is illustrative, not prescriptive. The point of the paper is the building blocks and their alternatives, not the order. The shipping order is a separate plan.

## 10. Recommendation

Build the meta-language. The §4 building blocks have small, locally-justifiable additions to existing machinery: `List<T>` extends the type system by one type, the literal extends the parser by one production, spread/HOFs/lambdas/pipe/reducers each touch the type checker but not the planner. Reflection is the substantial work and is rightly deferred.

The dbt parity claim is achievable. The dbt-exceedance claim — full LSP, type-checked HOFs, schema inference for dynamic columns, frame-stack diagnostics into meta — is what justifies the work over staying in `smelt.define`-only territory.

The three highest-leverage open decisions are:

- **Lambda syntax** (§4.5). Affects every HOF call site. The `=>` clash with named arguments has no costless solution. Lean: keyword-prefixed `fn x => body`, with positional disambiguation as backup.
- **Reflection API shape** (§4.8). Affects every §1 pattern that depends on workspace introspection. Lean: functional accessors under `smelt.*`, deferred to a follow-up spec, but type shape committed now.
- **Multi-model production mechanism** (§4.10.4). Affects every §4.10 pattern and is hard to undo once shipped — generated paths, frontmatter shape, LSP integration all hang off this decision. Lean: frontmatter directive `generates: models` + body returning `List<ModelDef>`; generated paths nested under the generator file by default, with a frontmatter override.

Everything else in §4 has a low-regret default that matches the existing language style. The §4.10 surface is bigger than §4.1–§4.9 combined; do not commit to it before the open questions in §8 (schema authoring overhead, generator-file naming, generated-path depth, multi-model identity stability under refactoring) have concrete answers.

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
