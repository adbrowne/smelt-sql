# Smelt Functions: Typed SQL Composition to Replace Jinja

**Date:** April 2026
**Status:** Research / Design Exploration
**Author:** Andrew Browne, with design input from Claude

This paper explores a design for **smelt functions**: typed, composable SQL fragments that replace Jinja macros while preserving smelt's static analysis guarantees. The core insight is that if the type system tracks *what kind of SQL fragment* a value is, composition can be free and still statically checked. We describe the type system, scoping model, gradual annotation strategy, planner integration, and an experimentation roadmap where each step teaches something that informs the next.

## 1. The Problem

smelt models are pure SQL with `smelt.ref()` / `smelt.source()` extensions. This covers orchestration (model dependencies, source declarations) and configuration (YAML frontmatter). It does not cover **logic reuse** -- the ability to define a pattern once and instantiate it across models with different inputs.

dbt solves this with Jinja macros, but at a steep cost: no type checking, no editor support, no planner visibility, and obscured logic from interleaving `{% for %}` / `{% if %}` with SQL.

Examining real dbt projects, Jinja macro usage falls into five categories:

| Category | Example | Smelt Status |
|----------|---------|-------------|
| **Expression reuse** | `{{ cents_to_dollars(amount) }}` | **Gap -- this paper** |
| **SQL fragment generation** | `{{ generate_surrogate_key(['col1', 'col2']) }}` | **Gap -- this paper** |
| **Whole-model templates** | `{{ generate_base_model(source('raw', 'orders')) }}` | **Gap -- this paper** |
| **Conditional SQL by environment** | `{% if target.name == 'prod' %}` | Solved by planner rules |
| **Variable / config access** | `{{ var('start_date') }}` | Solved by frontmatter + project config |

The gap is categories 1--3: reusable SQL at the expression, fragment, and model level. Jinja's fundamental failure is that it operates on strings. The type system cannot see through macros, the planner cannot reason about their semantics, and the LSP cannot provide completions inside macro bodies. If the type system tracks what kind of SQL fragment a value is, composition can be free and still statically checked.

## 2. Fragment Sorts -- The Core Idea

The design rests on **fragment sorts**: syntactic categories that distinguish different kinds of SQL fragments. This is the technique Rust's `macro_rules!` uses (fragment specifiers like `$e:expr`, `$s:stmt`), which derives from the PL concept of syntactic sorts in multi-sorted algebras. Rust proved this works at industrial scale. The key lesson: you need *enough* sorts to be useful but not so many that the system becomes its own type theory.

### The Sorts

| Sort | What it represents | Where it can appear |
|------|-------------------|-------------------|
| `Expr<T>` | Scalar expression of SQL type T | SELECT, WHERE, ON, HAVING, CASE, QUALIFY |
| `AggExpr<T>` | Expression containing aggregation | SELECT (with GROUP BY), HAVING |
| `TableExpr` | Something with a schema | FROM, JOIN, WITH |
| `SelectItems` | List of (expression, alias) pairs | SELECT clause |
| `Column<T>` | Column reference of type T | Anywhere Expr<T> is valid |
| `OrderSpec` | Expression + direction | ORDER BY |

These sorts ensure structural well-formedness: you cannot splice a `TableExpr` into a WHERE clause, or an `Expr<Boolean>` into a FROM clause. The compiler checks sort-correctness at each composition point.

`Column<T>` accepts only bare column references (`user_id`), not computed expressions (`LOWER(user_id)`). Use `Expr<T>` for computed values. This distinction has semantic weight when the parameter is spliced into `PARTITION BY` or `GROUP BY`.

A `Predicate` sort was considered and rejected. Use `Expr<Boolean>` instead -- the positional constraint (WHERE, ON, HAVING) is already enforced by SQL syntax, and one fewer concept to learn is worth more than one more sort.

The initial implementation targets a subset: `Expr<T>`, `TableExpr`, and `Column<T>`. The remaining sorts (`AggExpr<T>`, `SelectItems`, `OrderSpec`) are added once basic function composition is validated. Each sort is independently addable without breaking existing functions.

### Comparison: Malloy

Malloy is the closest direct competitor to this design. Malloy's `dimension` and `measure` declarations are essentially `Expr<T>` and `AggExpr<T>` with fixed scoping. Malloy's `source` extensions are `TableExpr` transformers. The key trade-off: Malloy gets cleaner semantics by abandoning SQL syntax; smelt gets migration compatibility by extending SQL but inherits SQL's scoping messiness. smelt's approach is less opinionated -- SQL fragments rather than a new semantic layer -- which makes incremental adoption easier for teams with large SQL codebases.

## 3. Functions over Fragments

Functions are defined with `smelt.define` and called via the `smelt.fn.*` namespace:

```sql
-- functions/core/safe_divide.sql
smelt.define safe_divide(
    numerator: Expr<Numeric>,
    denominator: Expr<Numeric>
) -> Expr<Double> AS (
    CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL
         ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
    END
)
```

```sql
-- Usage in a model
SELECT
    product_id,
    smelt.fn.safe_divide(total_revenue - total_cost, total_revenue) AS margin_pct
FROM smelt.ref('product_summary')
```

### Design Properties

**Functions compile away.** The target database engine never sees `smelt.fn.*` calls. Everything expands to plain SQL before execution. This is one-stage metaprogramming -- the same framing as Terra (Devito et al.) and MetaML (Taha & Sheard), where a high-level composition language generates low-level code.

**Named parameters follow PostgreSQL.** The `param => value` syntax follows PostgreSQL's named notation for function calls (supported since PostgreSQL 9.5). Oracle PL/SQL uses the same `=>` convention.

**No recursion.** Functions cannot call themselves, directly or indirectly. This guarantees termination -- the same totality property that makes Dhall work as a configuration language. Dhall demonstrated that a non-Turing-complete language with a modest type system can replace complex templating in practice. smelt's type system (SQL types plus fragment sorts) is the equivalent of Dhall's records and unions.

**Composition is free.** Functions can call other functions to any depth. A calls B calls C. The only restriction is no cycles, which guarantees finite expansion:

```sql
smelt.define sessionize(
    source: TableExpr,
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
    SELECT source.*,
        SUM(CASE WHEN ts_col - LAG(ts_col)
            OVER (PARTITION BY user_col ORDER BY ts_col)
            > gap THEN 1 ELSE 0 END)
        OVER (PARTITION BY user_col ORDER BY ts_col) AS session_id
    FROM source
)
```

### Conventions

**Namespacing is directory-derived.** Function paths under `smelt.fn.*` mirror the directory layout under `functions/`. `functions/patterns/session_rollup.sql` defines `smelt.fn.patterns.session_rollup`. This matches the `models/` convention.

**All functions are public.** No `pub` / `private` distinction in v1. Adding visibility modifiers later is non-breaking (default stays public).

**No overloading.** Function names are unique within their namespace. Overloading combined with gradual typing is a known footgun (resolution rules become annotation-tier-dependent).

**Functions are additive.** Introducing functions does not change the meaning of existing models. `smelt.define` and `smelt.fn.*` are purely additive to the grammar.

**Default values are self-contained.** A default expression cannot reference other parameters (keeps evaluation order trivial). Defaults are type-checked at definition time. For fragment-typed parameters, omitting the argument means "splice nothing" -- no special empty-list syntax needed. `Expr<Boolean>` filter parameters that should default to "no filter" use `= TRUE`.

**Function files use frontmatter** (same as model files). A `.sql` file may contain multiple `smelt.define` definitions. A file may also contain both definitions and a model `SELECT`. Conflicts (same function name in same directory) are reported as errors.

**Ordering is unrestricted.** A function may call any other function defined anywhere in the project, regardless of file or position within a file. Cycle detection lives in `smelt-db` as a Salsa query over the function-call graph.

## 4. Context Bindings -- Controlling What Callers Can See

When a function takes a fragment-typed parameter that contains column references, a natural question arises: which columns can it reference? The answer is **context bindings** -- optional annotations that declare which table context a fragment's columns resolve against.

Any fragment sort that can contain column references may declare a context:

| Sort | With context binding | Meaning |
|------|---------------------|---------|
| `Column<T>` | `Column<source, T>` | A column from `source` of SQL type T |
| `Expr<Boolean>` | `Expr<Boolean, source>` | Boolean expression whose columns come from `source` |
| `SelectItems` | `SelectItems<Agg, sessionized>` | Aggregate select items over `sessionized` columns |
| `Expr<T>` | `Expr<T, source>` | Scalar expression whose columns come from `source` |
| `OrderSpec` | `OrderSpec<enriched>` | Ordering expression over `enriched` columns |

A context can be:
1. **A `TableExpr` parameter** -- e.g., `Column<source>` where `source` is a parameter
2. **A CTE defined in the function body** -- e.g., `SelectItems<Agg, sessionized>` where `sessionized` is a `WITH` clause
3. **A union of contexts** -- e.g., `Expr<Boolean, source | customers>` for fragments referencing columns from multiple tables

### The Key Insight: Asymmetric Access Control

Consider `session_rollup`:

```sql
smelt.define session_rollup(
    source: TableExpr,
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean, source> = TRUE
) -> TableExpr @deterministic AS (
    WITH sessionized AS (
        smelt.fn.sessionize(source, user_col, ts_col, gap)
    )
    SELECT
        user_col, session_id,
        MIN(ts_col) AS session_start, MAX(ts_col) AS session_end,
        COUNT(*) AS event_count,
        metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_col, session_id
)
```

`metrics` binds to `sessionized` (the caller can reference `session_id` in their aggregate expressions), while `filters` binds to `source` (the author restricts filtering to raw source columns only). **The author controls what each caller-provided fragment can see.** This is capability-based access control applied to SQL column namespaces: the function author grants each parameter access to specific table contexts. Narrowing the context is how authors prevent callers from depending on internal implementation details.

### CTE-Derived Contexts

CTE context is computed, not declared. The author references a CTE by name in the type annotation; the compiler derives its schema from the body. This means the signature and body are coupled -- changing the CTE's SELECT list may change what callers can reference. This coupling is intentional: the context binding names the *splice-point scope*, and the splice point is in the body.

Computing the output schema of a CTE uses the same schema inference the type system already performs for models and function calls. The incremental work is wiring CTE schema computation into the context binding checker, not building a new analysis from scratch.

### Union Contexts for Joins

When a function joins multiple tables, the author can expose a union context:

```sql
smelt.define enrich_order(
    source: TableExpr,
    customer_id_col: Column<source, Integer>,
    product_id_col: Column<source, Integer>,
    extra_cols: SelectItems<source | customers | products> = ()
) -> TableExpr AS (
    WITH
        customers AS (SELECT * FROM smelt.ref('dim_customers')),
        products AS (SELECT * FROM smelt.ref('dim_products'))
    SELECT
        source.*,
        c.segment AS customer_segment,
        c.country AS customer_country,
        p.category AS product_category,
        extra_cols
    FROM source
    LEFT JOIN customers c ON source.customer_id_col = c.customer_id
    LEFT JOIN products p ON source.product_id_col = p.product_id
)
```

Ambiguous column names (present in multiple contexts) require qualification -- the same rule as standard SQL.

### Open Edge Cases

Union context disambiguation needs further specification. When both `a` and `b` have an `id` column, does the caller write `a.id`? The natural answer is parameter-name qualification, but the rules for parameter-CTE unions need to be made explicit.

CTE context checking is partially call-site-dependent: `SelectItems<Agg, sessionized>` can be structurally checked (is it a select list of aggregates?) at definition time, but column-name validation requires knowing the call-site schema when the CTE includes `source.*`.

Context bindings are **always optional.** Without them, column resolution happens at expansion time (Tier 1 behavior). Authors add them to shift checking earlier and give callers better errors.

## 5. Hybrid Scoping -- Lexical Parameters + Structural Column Resolution

When a function body says `user_col`, does it mean the parameter or a literal column named `user_col`?

The scoping model has two layers, reflecting the fact that SQL fragments inherently reference columns from table contexts:

**Layer 1 -- Lexical scoping for parameters.** Function parameters are explicit bindings. Inside a function body, `user_col` refers to whatever column the caller passed -- not a literal column in any ambient table. The compiler substitutes the actual column reference during expansion. This is **hygienic expansion** (Kohlbecker et al., 1986) -- like Rust macros, not C preprocessor macros.

**Layer 2 -- Structural column resolution within table contexts.** Bare column names in SQL expressions resolve against the schemas of `TableExpr` parameters in scope. This is unavoidable -- SQL is structurally scoped against its FROM clause:

```sql
smelt.define add_margin(source: TableExpr) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin
    FROM source
)
```

Here `revenue` and `cost` are not parameters -- they resolve from whatever schema `source` carries. This is **row polymorphism** (Remy, 1994; OCaml object types; PureScript row types): the function body is polymorphic over any table that has columns named `revenue` and `cost` of compatible types.

Column requirements can be declared explicitly:

```sql
smelt.define add_margin(
    source: TableExpr<{revenue: Numeric, cost: Numeric}>
) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin
    FROM source
)
```

With the annotation, the compiler checks requirements at the call site before expansion -- the caller gets "table passed to `source` is missing required column `revenue: Numeric`" rather than a post-expansion SQL error. Adding `..r` (`TableExpr<{revenue: Numeric, cost: Numeric, ..r}>`) further allows threading the caller's extra columns through to the return type.

**The honest description:** "Parameters are lexically scoped; column resolution within table-typed parameters is structural (schema-checked but not name-bound). Annotations make the structural requirements explicit."

**Why hybrid rather than pure lexical:** Functions are self-contained (readable without knowing the call site). The compiler can check bodies in isolation (when annotated). No surprises from ambient column names shadowing parameters. And the structural part is SQL-native for the parts that are inherently SQL. The two-layer model is more complex to explain than "everything is lexical," but it matches how SQL actually works.

## 6. Gradual Typing -- Three Tiers of Annotation

Mandatory type annotations would kill adoption among data analysts and analytics engineers. smelt follows TypeScript's gradual typing trajectory (Siek & Taha, 2006): start untyped, add types as code matures and gets shared.

### Tier 1: Unannotated (quick and personal)

```sql
smelt.define my_margin(revenue, cost) AS (
    CASE WHEN cost = 0 THEN NULL
         ELSE (revenue - cost) / cost
    END
)
```

No types declared. The compiler expands at each call site and type-checks the result. Good for personal utilities and prototyping.

### Tier 2: Parameters annotated (production code)

```sql
smelt.define my_margin(
    revenue: Expr<Numeric>,
    cost: Expr<Numeric>
) AS (
    CASE WHEN cost = 0 THEN NULL
         ELSE (revenue - cost) / cost
    END
)
```

Parameters typed, return type inferred. The compiler checks the body against parameter types **in isolation** -- no call site needed. Also checks each call site against declared types. Good for shared team code.

### Tier 3: Fully annotated (library quality)

```sql
smelt.define my_margin(
    revenue: Expr<Numeric>,
    cost: Expr<Numeric>
) -> Expr<Double> AS (
    CASE WHEN cost = 0 THEN NULL
         ELSE CAST(revenue - cost AS DOUBLE) / CAST(cost AS DOUBLE)
    END
)
```

Fully annotated. Checked in isolation. The LSP shows the return type on hover without expanding. Body-level errors are the *author's* problem, never shown to callers. Good for published packages.

### Error Message Contract

Error quality determines adoption. Each tier has a specific contract for what the *function user* sees:

**Tier 1:** The compiler expands, checks, and traces errors back to the call site with parameter mapping:

```
error: type mismatch in expression
  --> models/metrics.sql:5:12
   |
 5 |     smelt.fn.my_margin('hello', 'world')
   |                        ^^^^^^^ expected Numeric, got Text
   |
   = note: in expansion of `my_margin`, parameter `revenue` was bound to 'hello'
   = note: function defined at functions/my_margin.sql:1
```

Even though expansion happens before checking, the error maps back through the expansion to show the call site. This is better than C++ template errors because the expansion is structured (not arbitrary text substitution), so the trace is always possible.

**Tier 2:** Errors reference the function's declared parameter types, with no expansion needed:

```
error: type mismatch in argument
  --> models/metrics.sql:5:12
   |
 5 |     smelt.fn.my_margin(user_name, total_cost)
   |                        ^^^^^^^^^ expected Expr<Numeric>, got Expr<Text>
   |
   = note: parameter `revenue` declared as Expr<Numeric>
           at functions/my_margin.sql:2
```

**Tier 3:** Same call-site errors as Tier 2, plus hover types in the LSP.

**The principle:** As annotation tier increases, errors move earlier (call site vs. expansion), get shorter (declared type vs. traced binding), and shift responsibility toward the function author. The author bears the complexity of type annotations so that function users get clean error messages -- the same trade-off as Rust trait bounds.

### LSP Stability Under Broken Bodies

Tier 2/3 functions retain their signature when the body becomes invalid mid-edit. Call sites continue to type-check against declared types. Tier 1 functions have no signature independent of the body, so call sites cannot be checked while the body is broken -- a reason to encourage Tier 2+ for shared functions.

### Implementation Phasing

Tier 1 ships first -- it's just expansion plus existing type checking with error tracing. Tier 2 adds checking-mode verification on bodies. Tier 3 adds return type verification. Each tier is independently shippable. The type inference algorithm (next section) maps directly onto this phasing.

## 7. Type Inference -- Bidirectional Checking

smelt functions use **bidirectional type checking** (Pierce, 2004; Dunfield & Krishnaswami, 2021) with a local unification step at row-variable binding sites.

### Why Bidirectional

Three algorithm families were considered:

**Bidirectional checking (chosen):** Types flow in two directions -- "checking" mode pushes an expected type *down* into an expression, "synthesis" mode computes a type *up* from an expression. At function call sites, the declared parameter type is pushed down; the argument is checked against it. At function bodies, parameter types are pushed in, the body synthesizes a result, and the return annotation (if present) provides a checking target.

**Hindley-Milner / Algorithm W:** Overkill. smelt has no higher-order functions, no lambdas, no let-bindings where generalization matters. The only polymorphism is row polymorphism on parameters. Worse, HM's global constraint solving produces *non-local* error messages: when unification fails, the error references two constraints that are individually fine but jointly contradictory, far from where the conflict surfaces. This is the famously poor ML/Haskell error experience.

**Constraint-based with ranked heuristics (TypeScript/Scala 3 style):** Interesting but premature. The error quality advantage comes from ranking complex type relationships. smelt's type relationships are simple enough that bidirectional checking produces good errors without ranking.

### How It Maps to the Three Tiers

**Tier 1:** Expand the function, then run the checker on the expanded SQL in pure synthesis mode -- types flow up from leaves. Errors are mapped back to call-site parameter bindings via the expansion trace.

**Tier 2:** At the call site, push each parameter type into the corresponding argument in checking mode. If the parameter has a row variable (`Struct<{ts: Timestamp, ..r}>`), perform local unification against the concrete argument type -- this binds `r` immediately. The function body is checked in isolation: parameter types pushed in, body synthesized bottom-up.

**Tier 2+ (row variable in return type):** After binding `r` at the parameter, substitute into the return type. Downstream checking uses the concrete type. Error messages never mention row variables -- the user sees fully resolved types.

**Tier 3:** Check the body against the return type in checking mode. Row variables in the return type are still abstract (not bound to a concrete call), so the checker verifies the body produces *at least* the declared fields.

### Row Unification Is Local

Row-variable unification happens **at the point of use**, not via global constraint solving:

1. Match declared fields against concrete fields (check names and types).
2. Bind the row variable to the *remainder*.
3. Substitute the binding forward into other uses of that row variable.

This is strictly simpler than full HM unification: no union-find data structure, no occurs-check, no let-generalization, no global constraint propagation. Each call site is checked independently.

### Error Message Guarantees

The algorithm guarantees:

1. **Errors are always local.** Every error references a specific source location. No "constraint X from line 5 conflicts with constraint Y from line 20."
2. **Row variables never appear in user-facing errors.** By the time an error occurs, the variable is either bound (error shows the concrete type) or the binding failed (error shows "expected field X, struct has: {concrete fields}").
3. **Errors say "expected X, got Y."** Every type mismatch has two sides. This is the Rust/TypeScript error experience.
4. **Tier escalation improves locality without changing format.** Moving from Tier 1 to Tier 2 moves errors closer to the source without changing the error shape.

### What This Rules Out

- **Cross-boundary inference.** A Tier 1 function's return type is computed at each call site by expansion, not inferred from the body and propagated. Cross-boundary inference creates non-local errors. Functions that want stable types declare them (Tier 3).
- **Higher-rank polymorphism.** A parameter cannot itself be polymorphic. smelt functions are not higher-order.
- **Implicit subtyping coercions.** The checker does not silently insert casts. If a parameter expects `Expr<Double>` and the caller passes `Expr<Integer>`, this is a type error. The user writes `CAST(x AS DOUBLE)`. (Exception: engine aliases like `Text`/`Varchar` are treated as the same type.)

## 8. Block Syntax -- Ergonomic Fragment Passing

Passing multi-line SQL fragments as inline function arguments is syntactically awkward. Block syntax provides named `WITH ... AS (...)` clauses trailing a function call:

```sql
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_col => user_id,
    ts_col => event_timestamp,
    gap => INTERVAL '20 minutes'
)
WITH metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages,
    smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event
)
WITH filters AS (
    event_type != 'bot' AND user_id IS NOT NULL
)
```

Each `WITH name AS (...)` clause binds a fragment-typed parameter by name. The compiler treats it identically to inline arguments. The syntax reuses SQL's existing `WITH ... AS (...)` shape -- no significant whitespace, no nested mini-grammar, and the parser already knows how to handle parenthesized expressions after `AS`. Block clauses must trail the function call's closing `)` directly.

### Blocks Compose

A function can receive blocks from its caller and pass them through:

```sql
smelt.define monitored_session_rollup(
    source: TableExpr,
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
    metrics: SelectItems<Agg> = (),
    alerts: SelectItems<Agg, base> = ()
) -> TableExpr AS (
    WITH base AS (
        smelt.fn.session_rollup(source, user_col, ts_col)
        WITH metrics AS (metrics)
    )
    SELECT base.*,
        alerts
    FROM base
)
```

### Trade-offs

Block syntax introduces a second grammar layer at the call site. This has worked before (Ruby blocks, Kotlin DSL builders, Groovy closures in Gradle), but each time it creates parsing and tooling burden. Specific concerns:

- Error recovery inside blocks requires nested parsing contexts (SQL -> function call -> block -> SQL inside block).
- LSP contextual completion inside blocks depends on partial expansion -- one of the hardest LSP problems.

The choice to reuse SQL's existing `WITH ... AS (...)` shape mitigates parser complexity vs. inventing a wholly new block syntax. Named parameters with parenthesized fragments (the "ugly" version from the examples) remain available as the fallback -- blocks are pure sugar.

## 9. Row Polymorphism for Struct Values

The hybrid scoping model (section 5) handles row polymorphism for `TableExpr` parameters -- "this function works on any table with at least columns X, Y." But struct-typed columns (DuckDB, Spark, BigQuery) face the same brittleness problem at the *value* level: if struct parameters are closed, adding a field to the struct breaks every function that accepts it.

| | `TableExpr` | `Expr<Struct<{...}>>` |
|---|---|---|
| What it represents | A table/relation | A single struct-typed expression |
| Where it appears | FROM clause | Any expression position |
| Field access | Bare column names (`ts`, `user_id`) | Dot syntax (`event.ts`, `event.user_id`) |
| Expansion | Table reference substitution | Expression substitution with field access |

The PLT concept is the same -- row variables standing for "plus any other fields" -- but the surface types, field-access syntax, and compilation models differ.

### Open Struct Types

Extend `Struct<{...}>` with a **row variable** that stands in for "plus any other fields":

```text
Struct<{ ts: Timestamp, user_id: Text, ..r }>
```

`..r` is a named row variable bound at the function signature. `..` (anonymous) creates a fresh variable per parameter.

### Examples

**Reading fields from a struct column:**

```sql
smelt.define event_hour(
    event: Expr<Struct<{ts: Timestamp, ..}>>
) -> Expr<Integer> AS (
    EXTRACT(HOUR FROM event.ts)
)
```

Accepts any struct with a `ts: Timestamp` field. No overloads, no wrapping.

**Returning a struct with pass-through fields:**

```sql
smelt.define with_hour(
    event: Expr<Struct<{ts: Timestamp, ..r}>>
) -> Expr<Struct<{hour: Integer, ..r}>> AS (
    {hour: EXTRACT(HOUR FROM event.ts), ..event}
)
```

The `..event` spread in the body is the value-level counterpart of the type-level `..r`. The same row variable appears in both positions, so the checker knows the output struct's extra fields are the input struct's extra fields. Calling `with_hour` on a `STRUCT(ts TIMESTAMP, user_id TEXT, page TEXT)` produces `STRUCT(hour INTEGER, ts TIMESTAMP, user_id TEXT, page TEXT)`.

### Compilation Model

Row-polymorphic struct parameters **erase at expansion.** The compiler knows the concrete struct type at the call site and generates explicit field references:

```sql
-- event_hour expands to:
SELECT EXTRACT(HOUR FROM event_data.ts) AS hour
FROM page_events;

-- with_hour expands to:
SELECT {'hour': EXTRACT(HOUR FROM event_data.ts),
        'ts': event_data.ts,
        'user_id': event_data.user_id,
        'page': event_data.page} AS enriched
FROM page_events;
```

Row variables are resolved at the call site. At definition time, the body is checked against declared fields only; the row variable is opaque (you cannot enumerate or reflect on `..r`). The `..event` spread expands to the caller's remaining fields in a deterministic order (declaration order, with declared return fields first).

The compiler must support the target engine's struct literal syntax (DuckDB uses `{'field': value, ...}`). If an engine lacks struct literals, functions that construct new structs cannot target that engine -- a backend capability error, not a type error.

### Constraints

One named row variable per function in v1. Multi-row cases like `merge(a: Expr<Struct<{..r}>>, b: Expr<Struct<{..s}>>) -> Expr<Struct<{..r, ..s}>>` are deferred. When added, the semantics will be disjoint union (PureScript-style: the checker errors if `r` and `s` overlap). The single-variable rule covers essentially all current analytics use cases.

If two parameters both declare `..r`, they are constrained to have the same extra fields (useful when two struct arguments must share a shape). Anonymous `..` creates a fresh variable per parameter and can never be referenced elsewhere.

No defaults on row-polymorphic parameters in v1.

## 10. Planner Integration -- Three Levels

This is where smelt functions differ most fundamentally from Jinja macros. In dbt, macros are expanded to text before anything sees them. In smelt, functions are **visible to the planner as first-class nodes** with typed interfaces and declared properties.

### Why Not Just Expand?

If functions were expanded to plain SQL before the planner runs, the planner loses semantic information. It sees `SUM(CASE WHEN ts - LAG(ts) OVER (...) > INTERVAL '30 minutes' THEN 1 ELSE 0 END) OVER (...)` instead of knowing "this is a session rollup." Pattern-matching on raw SQL to rediscover this structure is fragile. Keeping functions in the IR means planner rules match on **function names and properties**, not SQL patterns.

### Level 1: Logical -> Logical (pre-expansion)

Rules rewrite the logical DAG. Functions are nodes with rich typed interfaces. In PLT terms, function types carry **refinement types** (Rondon et al., 2008): structural invariants beyond the basic fragment sort.

The compiler analyzes function bodies and attaches structural metadata:
- **Column provenance map:** Which output columns come from which input tables
- **Join graph:** Which tables are joined, join type, cardinality
- **Declared properties:** `@deterministic`, `@idempotent`, `@append_only`

This metadata enables filter pushdown, function fusion, join elimination, and semantic validation -- all by reasoning about the typed interface, never pattern-matching on SQL.

In v1, metadata is **explicitly annotated** by function authors (`@joins(dim_customers LEFT 1:1)`, `@provenance(customer_segment -> dim_customers.segment)`) rather than automatically derived. Automatic derivation requires a full lineage analyzer -- a substantial compiler component. Explicit annotations let the planner integration ship without this, while keeping the door open to add automatic derivation later as a DX improvement. In practice, only "model template" functions benefit from planner-level optimization, so the annotation burden is concentrated on a small number of high-value functions.

However: v1 annotations are bare keywords only (`@deterministic`, `@idempotent`, `@append_only`). Structured annotations (`@joins(...)`, `@provenance(...)`) are deferred until the planner actually needs them. Annotations appear after the type, before `=` (for parameters) or before `AS` (for return types).

The three-level planner architecture parallels MLIR's (Multi-Level IR) progressive lowering through dialect levels, where each level carries different semantic information and rewrites happen at the appropriate abstraction.

### Level 2: Logical -> Physical (strategy selection and expansion)

Rules choose an execution strategy and expand functions into strategy-specific SQL. The expansion is not mechanical inlining -- it's guided by the strategy:

- **Full rebuild:** Expand body as-is.
- **Incremental append:** Expand with a temporal filter injected into the source scan.
- **Incremental merge:** Expand with affected-key detection and recomputation.

The function author writes the pure logical version. Planner rules produce strategy-specific expansions. The function's declared return type is the safety contract: the rule must produce the same output schema.

### Level 3: Physical -> Execution Plan

A single physical node becomes one or more concrete SQL statements:
- Incremental merge: `CREATE TEMP TABLE -> DELETE matching rows -> INSERT FROM temp -> DROP temp`
- Cross-engine: `Run query on Spark -> Write Parquet -> Read in DuckDB`
- Validated write: `Run query -> Check invariants -> Swap target table`

Function properties still matter: `@idempotent` tells Level 3 that retry is safe; `@deterministic` tells it re-execution produces the same result.

### Planner Integration Is Post-MVP

v1 is pure expansion -- no Level 1 planner rules. Properties are parsed and stored but not actively used. This lets the function system be validated independently before adding planner complexity.

## 11. Built-in Function Typing

Models call built-in SQL functions far more often than user functions. If built-ins carry the same fragment-typed signatures, every SQL call gets compile-time checking, hover types, and completion.

### What Fits With No New Machinery (~80%)

| Built-in shape | Signature |
|----------------|-----------|
| Pure scalar (`LOWER`, `ABS`, `LENGTH`) | `Expr<T1> -> Expr<T2>` |
| Binary scalar (`POWER`, `MOD`) | `(Expr<T>, Expr<T>) -> Expr<T>` |
| Aggregates (`SUM`, `COUNT`, `AVG`) | `Expr<T> -> AggExpr<T>` |
| Predicate-producing (`IS NULL`, `LIKE`) | `Expr<T> -> Expr<Boolean>` |
| Simple table functions (`generate_series`) | `(Expr<Int>, Expr<Int>) -> TableExpr` |

### What Needs Extensions (~20%)

1. **Generics / type parameters.** `COALESCE(a, b, c)` returns the common supertype. `ARRAY_AGG(x)` returns `Array<T>`. Requires type parameters on signatures.
2. **Variadics.** `COALESCE`, `CONCAT`, `GREATEST` accept arbitrary arity. v1 excluded variadic user functions. For built-ins: either a privileged native-variadic form or reintroduce variadics for both.
3. **Types as arguments.** `CAST(x AS INTEGER)`, `EXTRACT(YEAR FROM ts)`. Not expressible as `Expr<T>`. Options: a `Type`/`Field` parameter sort, or primitive grammar handling.
4. **Keyword-argument syntax.** `TRIM(BOTH ' ' FROM x)`, `SUBSTRING(s FROM 1 FOR 3)`. Must be treated as primitive grammar.
5. **Modifier clauses.** `SUM(x) FILTER (WHERE cond)`, `OVER (...)`. Syntactic suffixes on aggregate calls.
6. **Schema-returning table functions.** `UNNEST(array_col)` depends on element type. `read_csv` with auto-schema is not compile-time typeable.

### What Is Untypeable

- Auto-schema built-ins without a schema hint (`read_csv('x.csv')`)
- Dynamic `EXECUTE` / string-templated SQL
- Untyped JSON navigation (`col->>'foo'`) -- typeable only as unconditional `Text`

### Positioning

v1 types only user functions and leaves built-ins to existing inference. v2 extends coverage. The extension is bounded and does not invalidate the existing design.

Open question: signature registry per dialect vs. primitive built-in shapes in the checker. The registry scales with engines; primitive shapes keep the checker simple.

## 12. Concrete Examples

### Example 1: safe_divide -- Expression Function

The simplest case. A reusable expression function.

**Definition:**
```sql
-- functions/core/safe_divide.sql
smelt.define safe_divide(
    numerator: Expr<Numeric>,
    denominator: Expr<Numeric>
) -> Expr<Double> AS (
    CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL
         ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
    END
)
```

**Usage:**
```sql
SELECT
    product_id,
    total_revenue,
    total_cost,
    smelt.fn.safe_divide(total_revenue - total_cost, total_revenue) AS margin_pct,
    smelt.fn.safe_divide(total_revenue, units_sold) AS revenue_per_unit
FROM smelt.ref('product_summary')
```

**What the compiler does:**
1. Verifies `total_revenue - total_cost` and `total_revenue` are numeric
2. Expands to `CASE WHEN total_revenue = 0 OR total_revenue IS NULL THEN NULL ELSE CAST((total_revenue - total_cost) AS DOUBLE) / CAST(total_revenue AS DOUBLE) END`
3. Infers return type as `DOUBLE` (nullable)

**What the LSP does:**
- Hover on `safe_divide` shows: `(Numeric, Numeric) -> Double?`
- Go-to-definition jumps to `functions/core/safe_divide.sql`
- Completions inside the call show available columns from `product_summary`

### Example 2: Session Rollup with Blocks

A reusable model pattern demonstrating composition, block syntax, context bindings, and planner integration.

**Usage:**
```sql
-- models/web_sessions.sql
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_col => user_id,
    ts_col => event_timestamp,
    gap => INTERVAL '20 minutes'
)
WITH metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages,
    smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event
)
WITH filters AS (
    event_type != 'bot' AND user_id IS NOT NULL
)
```

**Planner walkthrough:**

*Level 1 (logical -> logical):* The planner sees a `session_rollup` node. Checks that `web_events` is append-only and `event_timestamp` is a timestamp. Both pass.

*Level 2 (logical -> physical):* Selects incremental-append strategy. Expands with temporal filter: `FROM web_events WHERE event_timestamp > :watermark`.

*Level 3 (physical -> execution):*
1. `CREATE TEMP TABLE __staging AS (SELECT ... WHERE event_timestamp > :last_watermark)`
2. `DELETE FROM web_sessions WHERE session_id IN (SELECT session_id FROM __staging)` -- handles late events
3. `INSERT INTO web_sessions SELECT * FROM __staging`
4. Update watermark

Properties flow through all three levels: `@deterministic` tells Level 3 replaying a failed batch is safe; `@append_only` on `source` tells Level 2 incremental processing is valid.

### Example 3: Join Elimination via Function-Aware Planning

This demonstrates why planner-visible functions enable optimizations that blind expansion cannot.

**Setup:** `enrich_order` (defined in section 4) joins a fact table to customer and product dimensions via LEFT JOINs with unique keys (1:1 cardinality).

**Model A uses both dimensions:**
```sql
SELECT
    customer_segment, product_category,
    SUM(amount) AS total_revenue
FROM smelt.fn.enrich_order(
    source => smelt.ref('orders'),
    customer_id_col => customer_id,
    product_id_col => product_id
)
GROUP BY customer_segment, product_category
```

Both joins needed. No optimization.

**Model B uses only customer columns:**
```sql
SELECT
    customer_segment, customer_country,
    SUM(amount) AS total_revenue,
    COUNT(*) AS order_count
FROM smelt.fn.enrich_order(
    source => smelt.ref('orders'),
    customer_id_col => customer_id,
    product_id_col => product_id
)
GROUP BY customer_segment, customer_country
```

No columns from `dim_products` are used. The planner eliminates the join entirely:

```
rule eliminate_unused_1to1_left_join:
    match: function F containing LEFT JOIN to table T
    when:
        - no column from T is used by any downstream consumer of F
        - T's join key is declared unique
    then:
        rewrite F to remove the JOIN to T
```

This works because:
1. **Column provenance is explicit.** The typed interface tells the planner which columns come from which table.
2. **Join cardinality is known.** LEFT JOIN with unique key means 1:1 -- removing the join doesn't change row count.
3. **Downstream column usage is known.** The typed logical CST tells the planner exactly which columns are consumed.

With blind expansion, the planner would need to pattern-match raw SQL to identify JOINs, trace column lineage through aliases, check uniqueness constraints, and determine downstream usage. Steps 2-3 are fragile and break with SQL variations. With function-aware planning, the planner reads provenance from the typed interface.

## 13. Limits of the Design

Even at maximum ambition, some things remain outside scope:

- **Dynamic schema construction.** You cannot write a function that takes column names as runtime strings and produces a SELECT with those columns. The set of columns must be known at compile time. (`SelectItems` parameters cover the "list of things" case without requiring variadics.)
- **Conditional structure.** A function cannot return a JOIN sometimes and a subquery other times based on a runtime value. SQL structure is fixed at compile time. (Conditional *expressions* like CASE/WHEN are fine.)
- **Runtime parameterization of sorts.** Fragment type parameters are compile-time. You can pass runtime values as `Expr` (e.g., `WHERE col > smelt.param('cutoff')`), but you can't choose between different SQL structures at runtime.
- **Recursive patterns.** No function can call itself. Recursive CTEs remain a SQL-level feature, not a function-level one.

These limitations are deliberate. The Jinja use cases that hit them are exactly the ones that produce unmaintainable code.

## 14. Comparisons and Theoretical Foundations

### Comparison Table

| System | Approach | Lesson for smelt |
|--------|----------|-----------------|
| **dbt Jinja macros** | Untyped text substitution | Solves reuse but destroys analyzability. smelt must be the opposite. |
| **Rust `macro_rules!`** | Hygienic, fragment-sorted (`$e:expr`, `$t:ty`) | Proof that fragment sorts work at scale. Our `Expr<T>` is their `$e:expr`. |
| **Dhall** | Total, typed, no side effects | Totality from no-recursion. Modest type system covers 95% of cases. |
| **C++ templates** | Untyped expansion, check at instantiation | Good tracing can compensate for late checking (Tier 1). But early checking is better. |
| **TypeScript** | Gradual typing, strict optional | Adoption trajectory: start untyped, add types as code matures. |
| **Malloy** | First-class dimensions, measures, sources | Closest direct competitor. Cleaner semantics by abandoning SQL; smelt is more migration-friendly by extending SQL. Malloy's `dimension`/`measure` = our `Expr<T>`/`AggExpr<T>` with fixed scoping. |
| **PRQL** | Functions as first-class values, pipeline syntax | Functions over expressions work well. But PRQL is a whole new language. |
| **Terra** | Staged programming -- generate low-level code from high-level | "Generate SQL from a composition language" is exactly this framing. |

### Theoretical Underpinnings

The design draws from several established PL techniques:

- **Fragment sorts / syntactic categories.** Multi-sorted algebras; Rust `macro_rules!` fragment specifiers. The foundation of safe composition.
- **Staged metaprogramming.** MetaML (Taha & Sheard); Terra. Functions that "compile away" are one-stage programming.
- **Hygienic macro expansion.** Kohlbecker et al., 1986. Lexical scoping for parameters prevents C-preprocessor-style surprises.
- **Row polymorphism.** Remy, 1994; OCaml object types; PureScript row types. Structural column/field resolution for tables and structs.
- **Gradual typing.** Siek & Taha, 2006. Optional annotations with a clean adoption trajectory.
- **Totality via structural restriction.** Turner, 2004. No recursion guarantees termination.
- **Refinement types.** Rondon et al., 2008 (liquid types). Function types carry structural invariants (provenance, join graphs) beyond the basic sort.
- **Bidirectional type checking.** Pierce, 2004; Dunfield & Krishnaswami, 2021. Types flow up (synthesis) and down (checking).
- **Progressive lowering.** MLIR. Three planner levels with clear contracts at each boundary.

### Historical Precedents Worth Studying

- **SML Functors / OCaml modules** -- Parameterized modules that produce types based on input types. `Column<source, T>` is a simplified version.
- **Scala's path-dependent types** -- `source.Column` where the type depends on a specific value. Context bindings are more constrained, avoiding Scala's complexity.
- **Template Haskell** -- Multi-stage compilation where generated code is type-checked after splicing. Tier 1 is exactly this. TH showed that error messages are the main usability challenge.
- **Liquid types (Rondon et al., 2008)** -- Refinement types carrying logical predicates. The compiler-derived structural metadata is a domain-specific form.
- **Heeren et al. (2003), "Top Quality Type Error Messages"** -- Analysis of why constraint-based systems produce poor errors. A "what to avoid" reference justifying bidirectional checking.

## 15. Open Questions

### Specification to Tighten

- **Union context disambiguation.** For `Column<a | b, Integer>` when both have an `id` column, does the caller write `a.id`? Parameter-name qualification is the natural answer but needs explicit specification, including parameter-CTE union cases.
- **CTE context checking boundary.** `SelectItems<Agg, sessionized>` can be structurally checked at definition time, but column-name validation is call-site-dependent when the CTE includes `source.*`. The split needs clear documentation.
- **`AggExpr<T>` -- keep or collapse into `Expr<T>`?** Same argument as the Predicate removal: aggregation context is enforced by SQL syntax. Counter-argument: "this parameter expects an aggregate" is a common source of confusion. Deferred to implementation -- not in MVP scope either way.

### Deferred

- **LSP block-context completion** -- architecturally hard, can land after basic diagnostics.
- **Function tests** -- functions remain testable through models that use them. First-class function-test workflow is a follow-up.
- **Package ecosystem / registry** -- not in initial scope.
- **Python model interaction** -- functions are SQL-only; Python models are opaque table producers reachable via `smelt.ref()`.
- **Structured annotations** (`@joins(...)`, `@provenance(...)`) -- deferred until the planner needs them.
- **Error trace depth for nested calls** -- when A calls B calls C and C errors, Tier 1 shows A->C (call site -> innermost error), skipping intermediates. Full-chain traces are a future improvement.

## 16. Experimentation Roadmap -- What We Learn at Each Step

This is a research sequence, not a shipping plan. Each step teaches something that informs the next.

### Step 1: Fragment Sorts + Expr<T> Functions

**Build:** `smelt.define` for expression-level functions. `Expr<T>` and `Column<T>` sorts. Tier 1 checking (expand, check, trace errors back). The `safe_divide` example end-to-end.

**What we learn:** Does the fragment sort concept work in practice? Does expansion + type checking produce errors good enough for Tier 1? Is the `smelt.define` / `smelt.fn.*` syntax natural?

**How it ladders:** If Tier 1 errors are adequate, the gradual typing thesis holds. If not, we know exactly where the pain is before adding complexity.

### Step 2: TableExpr Functions + Row Polymorphism

**Build:** `TableExpr` sort. Structural column resolution (bare column names resolving against table schemas). The `sessionize` and `add_margin` examples.

**What we learn:** Does structural column resolution work? How do errors feel when a table is missing required columns? Is the hybrid scoping model (lexical parameters + structural columns) confusing or natural?

**How it ladders:** This validates the row polymorphism thesis. If structural resolution works for tables, the same concept extends to structs (Step 7).

### Step 3: Context Bindings

**Build:** Context parameters on fragment sorts. `SelectItems<Agg, sessionized>`, `Expr<Boolean, source>`. CTE-derived contexts. Union contexts.

**What we learn:** Can we derive CTE schemas reliably? Do context-bound errors actually help? Is the capability-based access control ("author controls what callers see") valuable in practice?

**How it ladders:** Context bindings are what make the error story qualitatively better than "just expand and check." This is the bridge from "it works" to "it works well."

### Step 4: Tier 2 Annotations + Bidirectional Checking

**Build:** Parameter type annotations. Bidirectional checking in synthesis and checking modes. Pre-expansion call-site checking.

**What we learn:** Does pre-expansion checking produce meaningfully better errors? Is bidirectional checking sufficient, or do we hit cases that want constraint-based solving? How much annotation do people actually write?

**How it ladders:** This is the inflection point for library-quality functions. If Tier 2 errors are good, Tier 3 (return type annotations) is a small incremental step.

### Step 5: Block Syntax

**Build:** Trailing `WITH name AS (...)` clauses on function calls. Parser integration and error recovery.

**What we learn:** Is the parser complexity manageable? Does the syntax actually improve readability over inline arguments? How does error recovery work inside blocks?

**How it ladders:** Block syntax is pure ergonomics. It does not change the type system. It can be added or deferred without affecting anything else. Can happen any time after Step 1.

### Step 6: Planner Visibility

**Build:** Functions as nodes in the logical plan. Explicit property annotations. Column provenance annotations. The join elimination example.

**What we learn:** Can planner rules reason about functions effectively? Does join elimination actually fire on real workloads? Is the explicit annotation burden acceptable, or is automatic derivation needed sooner than expected?

**How it ladders:** This is where smelt functions become fundamentally different from Jinja macros. Everything before this is "better macros." This is "optimization annotations."

### Step 7: Struct Row Polymorphism

**Build:** `Expr<Struct<{ts: Timestamp, ..r}>>`. Row variables on struct types. Spread syntax. Value-level erasure at expansion.

**What we learn:** Do analytics teams actually have struct-typed columns that benefit from this? Does the single-named-variable restriction bite? Can row-unification errors be explained clearly?

**How it ladders:** Validates whether row polymorphism generalizes from tables to values. If it does, the type system has broader reach than initially designed for.

### Step 8: Built-in Function Typing

**Build:** Signature registry or primitive shapes for SQL built-ins. Generics and variadics for the 20% that need extensions.

**What we learn:** Can the fragment sort system extend to cover SQL built-ins? Which extension categories (generics, variadics, type-as-arguments, keyword syntax, modifier clauses) have the best effort/value ratio?

**How it ladders:** This is the second-largest leverage point after the core function system. If it works, every SQL call in every model benefits from the type system.

### Dependency Graph

```
Step 1 -> Step 2 -> Step 3 -> Step 4   (sequential: each builds on the previous)
                                  |
Step 5 (independent, any time after Step 1)
                                  |
                         Step 6, 7, 8   (independent of each other, all need 1-4)
```
