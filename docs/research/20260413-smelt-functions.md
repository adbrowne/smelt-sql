# Smelt Functions: Typed SQL Composition to Replace Jinja

**Date:** April 13, 2026
**Status:** Discussion / Design Exploration
**Author:** Andrew Browne, with design input from Claude

## 1. Motivation

dbt's Jinja macros solve a real problem: SQL reuse. Common patterns like currency conversion, surrogate key generation, session rollups, and standard enrichment joins get copy-pasted across models without a composition mechanism. Jinja macros provide that mechanism — but at a steep cost:

- **No type checking.** A macro that expects a numeric column silently produces invalid SQL when given a string. Errors appear in the generated SQL, not the macro.
- **No editor support.** No go-to-definition for macro parameters, no completions inside macro bodies, no hover types.
- **No planner visibility.** Jinja expands to text before the SQL is parsed. The optimizer can't see through macros, can't reason about their semantics, can't apply macro-aware optimizations.
- **Obscured logic.** Interleaving `{% for %}`, `{% if %}`, and SQL makes both the template logic and the business logic harder to read.

smelt deliberately avoids Jinja, but the reuse problem remains. Today, smelt models are pure SQL with `smelt.ref()` / `smelt.source()` extensions. This covers orchestration (model dependencies, source declarations) and configuration (YAML frontmatter). It does not cover **logic reuse** — the ability to define a pattern once and instantiate it across models with different inputs.

This paper explores a design for **smelt functions**: typed, composable SQL fragments that replace Jinja macros while preserving smelt's static analysis guarantees.

### What Jinja Is Actually Used For

Examining real dbt projects, Jinja macro usage falls into five categories:

| Category | Example | Smelt Status |
|----------|---------|-------------|
| **Expression reuse** | `{{ cents_to_dollars(amount) }}` | **Gap — this paper** |
| **SQL fragment generation** | `{{ generate_surrogate_key(['col1', 'col2']) }}` | **Gap — this paper** |
| **Whole-model templates** | `{{ generate_base_model(source('raw', 'orders')) }}` | **Gap — this paper** |
| **Conditional SQL by environment** | `{% if target.name == 'prod' %}` | Solved by planner rules |
| **Variable / config access** | `{{ var('start_date') }}` | Solved by frontmatter + project config |

The gap is categories 1–3: reusable SQL at the expression, fragment, and model level.

## 2. Design Overview

The design has four layers, each building on the previous:

1. **SQL Fragment Types** — The type system distinguishes different kinds of SQL fragments (expressions, table expressions, select lists, predicates). This is the foundation that makes safe composition possible.

2. **Functions over fragments** — Users define functions (`smelt.define`) that take typed SQL fragments as parameters and return typed SQL fragments. These compose freely — a function can call other functions.

3. **Block syntax** — Ergonomic call-site syntax for passing multi-line SQL fragments to functions. Syntactic sugar over fragment-typed parameters.

4. **Three-level planner integration** — Functions are visible in the logical plan as first-class nodes. Planner rules can match on function names and properties, enabling function-aware optimization. Expansion to plain SQL happens late, guided by the planner's strategy decisions.

### Design Principles

- **Functions compile away.** The target database engine never sees `smelt.fn.*` calls. Everything expands to plain SQL before execution. Functions are a compile-time mechanism.
- **Lexical scoping.** Function parameters are explicit bindings, not ambient column references. This makes functions self-contained and analyzable in isolation.
- **Optional annotations.** Type annotations on parameters and return values are optional. Unannotated functions are checked at call sites (like C++ templates). Annotated functions are also checked in isolation (like Rust generics). Users start simple and add rigor as functions mature.
- **No recursion.** Functions cannot call themselves, directly or indirectly. This guarantees termination and makes expansion always finite.
- **Author complexity, user clarity.** Function authors bear the complexity of type annotations so that function users get clean error messages and a great editor experience. This parallels Rust traits: the author writes the bounds; the caller just passes arguments and gets clear errors like "expected Numeric, got Text." Annotations are always optional — but the more an author provides, the better the experience for every caller.
- **Planner transparency.** The planner can see function boundaries, match on function names/properties, and apply semantic optimizations. Functions are not just a user convenience — they are optimization annotations.

## 3. SQL Fragment Types

The core insight: Jinja defeats analysis because it operates on strings. If the type system tracks **what kind of SQL fragment** a value is, composition can be free and still statically checked.

### Fragment Sorts

| Sort | What it represents | Where it can appear |
|------|-------------------|-------------------|
| `Expr<T>` | Scalar expression of SQL type T | SELECT, WHERE, ON, HAVING, CASE, ORDER BY |
| `AggExpr<T>` | Expression containing aggregation | SELECT (with GROUP BY), HAVING, ORDER BY |
| `WindowExpr<T>` | Expression containing a window function | SELECT, ORDER BY |
| `TableExpr` | Something with a schema | FROM, JOIN, WITH |
| `SelectItems` | List of (expression, alias) pairs | SELECT clause |
| `Predicate` | Boolean expression | WHERE, ON, HAVING, QUALIFY |
| `OrderSpec` | Expression + direction | ORDER BY |

> **Note (April 18, 2026):** `Column<T>` was removed from the fragment sorts for v1. A bare column reference is a trivial `Expr<T>`. `Column<T>` can be reintroduced later as a subtype at the bottom of the expression chain (`Column<T> <: Expr<T> <: AggExpr<T> <: WindowExpr<T>`) if lineage tracking or planner needs require distinguishing bare column references from computed expressions. See §16 for rationale.

These sorts ensure structural well-formedness: you cannot splice a `TableExpr` into a WHERE clause, or a `Predicate` into a FROM clause. The compiler checks sort-correctness at each composition point.

### Expression Level Subtyping

The three expression sorts — `Expr<T>`, `AggExpr<T>`, and `WindowExpr<T>` — form a linear subtyping chain that mirrors SQL's evaluation order:

```
Expr<T>  <:  AggExpr<T>  <:  WindowExpr<T>
(scalar)     (may aggregate)  (may window)
```

**Why this is the right structure:** SQL evaluates in a fixed order: FROM → WHERE → GROUP BY → HAVING → **Window** → SELECT → ORDER BY. Each expression sort corresponds to an evaluation level. A pure scalar (`col + 1`) is valid at every level. An aggregate (`SUM(x)`) is valid at the aggregate level and above. A window expression (`SUM(x) OVER (...)`) is valid only at the window level. Subtyping follows directly: a value valid at a lower level is always valid at a higher level.

This means placement rules are just upper bounds on the expression level:

| SQL clause | Accepts up to | Meaning |
|------------|--------------|---------|
| WHERE, ON | `Expr<T>` | Scalars only — evaluated before grouping |
| HAVING | `AggExpr<T>` | Scalars and aggregates — evaluated after grouping |
| SELECT, ORDER BY | `WindowExpr<T>` | Anything — evaluated after windowing |

**For function authors**, the level on a parameter controls what callers can pass:

```sql
-- Only accepts pure scalars (no aggregates, no windows)
smelt.define safe_divide(numerator: Expr<Numeric>, ...) -> ...

-- Accepts scalars or aggregates (the function will place this in a GROUP BY context)
smelt.define rollup(..., metrics: AggExpr<Numeric>, ...) -> ...

-- Accepts anything including window expressions (the function places this directly in SELECT)
smelt.define report(..., computed: WindowExpr<Numeric>, ...) -> ...
```

The subtyping chain also extends to `SelectItems`. Today `SelectItems<Agg, ctx>` marks aggregate-level select items. This generalizes naturally: `SelectItems<Window, ctx>` accepts items at any expression level, `SelectItems<Agg, ctx>` accepts scalars and aggregates, and a hypothetical `SelectItems<Scalar, ctx>` would accept only pure scalars.

**No union types needed.** Because the expression sorts form a linear chain rather than a diamond, a function author always picks a single level — there is no case where you need `Expr<T> | WindowExpr<T>` but not `AggExpr<T>`. The subtyping handles every combination naturally.

### Table Context Bindings

Any fragment sort that can contain column references may optionally declare which **context** its columns resolve against.

> **Revised (April 18, 2026).** The original design used explicit context bindings in signatures and union contexts for joins. This has been replaced with an inference-based system. See §16 for the full context resolution design. The key changes: (1) context is inferred from splice points in the body, not declared in the signature; (2) union contexts are replaced by a no-overlap rule + `smelt.as_struct()` for namespacing; (3) context annotations in signatures are optional documentation, validated against inference.

The compiler derives the context for each parameter by analyzing where it is spliced in the function body. Context annotations in signatures are optional — when present, they serve as documentation and are validated against the inferred context.

| Sort | With context annotation | Meaning |
|------|------------------------|---------|
| `Predicate` | `Predicate<source>` | A boolean expression whose columns come from `source` |
| `SelectItems` | `SelectItems<Agg, sessionized>` | Aggregate select items over `sessionized` columns |
| `Expr<T>` | `Expr<T, source>` | A scalar expression whose columns come from `source` |
| `OrderSpec` | `OrderSpec<enriched>` | An ordering expression over `enriched` columns |

Without a context annotation, the compiler infers context from the splice point in the body. With a context annotation, the compiler validates it matches the inferred context and uses it for documentation (LSP hover, etc.).

```sql
smelt.define session_rollup(
    source: TableExpr,
    user_key: Expr<source>,                      -- expression over source columns
    ts_expr: Expr<source, Timestamp>,            -- timestamp expression from source
    gap: Expr<Interval> = INTERVAL '30 minutes', -- no table context (literal)
    metrics: SelectItems<Agg, sessionized> = (), -- agg over sessionized (source.* + session_id)
    filters: Predicate<source> = TRUE            -- predicate over source columns only
) -> TableExpr @deterministic AS (
    WITH sessionized AS (
        smelt.fn.sessionize(source, user_key, ts_expr, gap)
    )
    SELECT
        user_key, session_id,
        MIN(ts_expr) AS session_start, MAX(ts_expr) AS session_end,
        COUNT(*) AS event_count,
        metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_key, session_id
)
```

Note the deliberate asymmetry: `metrics` is spliced into a SELECT over `sessionized` (the caller can reference `session_id` in their aggregate expressions), while `filters` is spliced into a WHERE over `source` (the author restricts filtering to raw source columns only). **The author controls what each caller-provided fragment can see** by choosing where to splice each parameter.

#### Multi-Table Contexts and the No-Overlap Rule

> **Revised (April 18, 2026).** Union contexts (`Predicate<source | customers | products>`) have been replaced with a simpler system. See §16 decision 4.

When a function joins multiple tables, the context for a parameter spliced in the JOIN scope must have **no overlapping column names**. If `source` and `dim_customers` both have an `id` column, splicing a parameter into that JOIN scope is a compile error.

**Resolution options for the function author:**
1. Use explicit SELECT lists in CTEs to eliminate overlaps
2. Use typed `TableExpr` parameters to control the input schema
3. Use `smelt.as_struct()` to namespace each table's columns into a struct

```sql
smelt.define enrich_order(
    source: TableExpr,
    customer_id_expr: Expr<source, Integer>,
    product_id_expr: Expr<source, Integer>,
    extra_cols: SelectItems = ()
) -> TableExpr AS (
    SELECT
        source.*,
        smelt.as_struct(c EXCEPT customer_id) AS customer,
        smelt.as_struct(p EXCEPT product_id) AS product,
        extra_cols
    FROM source
    LEFT JOIN smelt.ref('dim_customers') c ON source.customer_id_expr = c.customer_id
    LEFT JOIN smelt.ref('dim_products') p ON source.product_id_expr = p.product_id
)
```

The caller references columns via struct field access:
```sql
extra_cols => customer.country, product.category, source.amount * 1.1 AS adj_amount
```

`smelt.as_struct(alias EXCEPT ...)` is a compile-time construct: `customer.country` expands to `c.country` in the generated SQL. Zero runtime cost. The EXCEPT clause removes columns (typically join keys) analogous to `SELECT * EXCEPT`.

#### Design Properties

Context annotations are **always optional.** A function author who omits them gets compiler-inferred context. An author who adds them gets validated documentation and clearer LSP hover information. This follows the **author complexity, user clarity** principle.

**Context is inferred, not declared.** The compiler analyzes the function body to determine which scope each parameter resolves against. When a parameter is used in multiple places, the context is the **intersection** of columns visible at all splice points. If the intersection is empty, it's a compile error. When a parameter is used in multiple different scopes, the context annotation becomes required — the author must explicitly specify which scope.

**CTE context is computed from the body.** The compiler derives the schema of each CTE from the body. This means the signature and body are coupled — changing a CTE's SELECT list may change what callers can reference. This coupling is intentional: the context represents the *splice-point scope*, and the splice point is in the body.

## 4. Function Definitions

### Syntax: `smelt.define`

Functions are defined in `.sql` files (likely in a `functions/` directory, location TBD):

```sql
-- Expression-level function
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
-- Table-level function (produces a full query)
smelt.define sessionize(
    source: TableExpr,
    user_key: Expr<source>,
    ts_expr: Expr<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
    SELECT source.*,
        SUM(CASE WHEN ts_expr - LAG(ts_expr)
            OVER (PARTITION BY user_key ORDER BY ts_expr)
            > gap THEN 1 ELSE 0 END)
        OVER (PARTITION BY user_key ORDER BY ts_expr) AS session_id
    FROM source
)
```

### Calling Convention

Functions are called via the `smelt.fn.*` namespace:

```sql
-- Expression function: inline in any expression context
SELECT smelt.fn.safe_divide(revenue, cost) AS margin
FROM smelt.ref('orders')

-- Table function: used as a table source
SELECT * FROM smelt.fn.sessionize(
    source => smelt.ref('events'),
    user_key => user_id,
    ts_expr => event_timestamp
)
```

### Composition

Functions can call other functions:

```sql
smelt.define session_rollup(
    source: TableExpr,
    user_key: Expr<source>,
    ts_expr: Expr<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    extra_metrics: SelectItems<Agg, sessionized> = ()
) -> TableExpr AS (
    WITH sessionized AS (
        smelt.fn.sessionize(source, user_key, ts_expr, gap)
    )
    SELECT
        user_key,
        session_id,
        MIN(ts_expr) AS session_start,
        MAX(ts_expr) AS session_end,
        COUNT(*) AS event_count,
        extra_metrics
    FROM sessionized
    GROUP BY user_key, session_id
)
```

There is no limit on composition depth. A calls B calls C. The only restriction is no cycles (no recursion), which guarantees finite expansion.

## 5. Block Syntax

### The Problem

Passing multi-line SQL fragments as function arguments is syntactically awkward:

```sql
-- This is hard to read
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('events'),
    user_key => user_id,
    ts_expr => event_timestamp,
    extra_metrics => (SUM(revenue) AS total_revenue, COUNT(DISTINCT page) AS unique_pages),
    filters => (event_type != 'bot' AND user_id IS NOT NULL)
)
```

### PASSING Syntax (decided April 18, 2026)

> **Revised (April 18, 2026).** The original `{ }` curly-brace block syntax has been replaced with `PASSING` clauses. See §16 decision 3 for rationale.

The `PASSING name AS (...)` clause after a function call provides named fragment-typed arguments:

```sql
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_key => user_id,
    ts_expr => event_timestamp,
    gap => INTERVAL '20 minutes'
)
PASSING metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages,
    smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event
)
PASSING filters AS (
    event_type != 'bot'
    AND user_id IS NOT NULL
)
```

This desugars to passing the PASSING contents as fragment-typed arguments. The compiler treats it identically to inline arguments. `PASSING` uses a distinct keyword (with SQL/XML precedent) to avoid ambiguity with `WITH` CTEs.

**Key properties:**
- `PASSING` blocks attach to the immediately preceding `smelt.fn.*` call
- Multiple `PASSING` clauses allowed per call (one per fragment parameter)
- Nested function calls use inline named args; `PASSING` only works at the outermost call level (in FROM position)
- Each section is delimited by parentheses — no whitespace sensitivity

### Blocks Compose

A function can receive blocks from its caller and pass them through to other functions:

```sql
smelt.define monitored_session_rollup(
    source: TableExpr,
    user_key: Expr<source>,
    ts_expr: Expr<source, Timestamp>,
    metrics: SelectItems<Agg> = (),   -- no context: passed through to session_rollup,
                                      -- which validates against its own context
    alerts: SelectItems<Agg, base> = ()
) -> TableExpr AS (
    WITH base AS (
        smelt.fn.session_rollup(source, user_key, ts_expr)
        PASSING metrics AS (metrics)   -- pass through caller's metrics
    )
    SELECT base.*,
        alerts
    FROM base
)
```

## 6. Optional Type Annotations

### Three Tiers

Following Rust's model of optional trait bounds, smelt functions support three levels of type annotation:

#### Tier 1: Unannotated (quick and personal)

```sql
smelt.define my_margin(revenue, cost) AS (
    CASE WHEN cost = 0 THEN NULL
         ELSE (revenue - cost) / cost
    END
)
```

No types declared. The compiler expands at each call site and type-checks the result. If someone calls `my_margin('hello', 'world')`, the error traces back to the call site.

**Checking strategy:** Expand, then check the plain SQL, then trace errors to the source location in the calling model.

**Good for:** Personal utilities, prototyping, one-off models.

#### Tier 2: Parameters annotated (production code)

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

Parameters typed, return type inferred. The compiler checks the body against parameter types **in isolation** — no call site needed.

**Checking strategy:** Check body against declared parameter types. Also check each call site.

**Good for:** Shared team code, models others depend on.

#### Tier 3: Fully annotated (library quality)

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

Fully annotated. Checked in isolation. The LSP shows the return type on hover without expanding.

**Checking strategy:** Check body against parameter AND return type. Check each call site against declared types.

**Good for:** Published packages, widely-shared functions.

### Why Optional

dbt's audience is data analysts and analytics engineers, not programming language enthusiasts. Mandatory `Expr<source, Timestamp>` annotations would kill adoption. Unannotated functions that "just work" let people get value immediately. Types become valuable as code matures and gets shared — the same trajectory as TypeScript's gradual typing adoption.

### Implementation Phasing

Tier 1 can ship first — it's just expansion + existing type checking with error tracing. Tier 2 adds constraint checking on bodies. Tier 3 adds return type verification. Each tier is independently shippable.

### Error Message Contract

Error quality determines adoption. Each tier has a specific error message contract — what the *function user* (not author) sees when something goes wrong:

#### Tier 1 errors (unannotated functions)

The compiler expands the function, type-checks the result, and traces errors back to the call site with parameter mapping:

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

The key: even though expansion happens before checking, the error message maps back through the expansion to show the call site and parameter binding. This is the minimum viable error experience — better than C++ templates, because the expansion is structured (not arbitrary text substitution) so the trace is always possible.

#### Tier 2 errors (parameters annotated)

The compiler checks at the call site *before expansion*. Errors reference the function's declared parameter types:

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

No expansion needed. The error is at the call site and references the declared contract. This is the Rust trait-bound experience: the author specified what they need, and the caller is told exactly which argument doesn't match.

#### Tier 3 errors (fully annotated)

Same call-site errors as Tier 2, plus the LSP can show return types on hover without expansion. The function body is also checked in isolation — body-level errors are the *author's* problem, never shown to callers.

**The principle:** As annotation tier increases, errors move earlier (call site vs. expansion), get shorter (declared type vs. traced binding), and shift responsibility toward the function author. This is the **author complexity, user clarity** principle in action.

## 7. Scoping: Lexical vs Dynamic

> **Revised (April 18, 2026).** The scoping rules have been refined with a clearer resolution order. See §16 decision 1.

### The Question

When a function body says `user_key`:

```sql
smelt.define sessionize(
    source: TableExpr,
    user_key: Expr<source>,
    ...
) -> TableExpr AS (
    SELECT ... PARTITION BY user_key ...
    FROM source
)
```

Does `user_key` mean the parameter (lexical scoping) or a literal column named `user_key` in the ambient table (dynamic scoping)?

### Decision: Hybrid Scoping with Parameters-First Resolution

The scoping model has two layers, reflecting the fact that SQL fragments inherently reference columns from table contexts:

**Layer 1 — Parameters always win.** Function parameters are explicit bindings. If a bare name matches a parameter, it resolves to the parameter. This is **hygienic expansion** — like Rust macros, not C preprocessor macros.

**Layer 2 — Standard SQL FROM-clause resolution.** After parameters, bare names resolve against the FROM/JOIN/CTE scope using normal SQL rules: single table in scope → bare names work; multiple tables → ambiguous names require qualification. This is closer to **row polymorphism** (as in OCaml's object types or PureScript's row types) than pure lexical scoping.

```sql
smelt.define add_margin(source: TableExpr) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin
    FROM source
)
```

Here `revenue` and `cost` are not parameters — they resolve from whatever schema `source` carries via normal SQL column resolution. The function body is polymorphic over any table that has columns named `revenue` and `cost` of compatible types.

**Shadow warning.** When a parameter name shadows a column in a `TableExpr` parameter's schema, the compiler emits a warning. This catches the case where adding a parameter to a function silently reinterprets a column reference. Detectable at the call site where the schema is known.

**The resolution order in function bodies:**
1. Is it a parameter name? → parameter (lexical binding)
2. Is it an unambiguous column in the FROM scope? → column (SQL resolution)
3. Ambiguous across multiple tables? → error (standard SQL ambiguity rules; also prevented by the no-overlap rule from §16 decision 4)

**Models vs function bodies:** In regular model files (not function definitions), bare column names work as normal SQL. No parameters exist, so there's no ambiguity. The parameter-first rule only applies inside `smelt.define` bodies.

**Rationale:**
- Functions are **self-contained** — readable without knowing the call site
- The compiler can **check the body in isolation** (when annotated)
- **Parameters take priority** — adding a column to a source table cannot silently change function behavior (the shadow warning catches this)
- **SQL-native** for the column resolution layer — developers use existing SQL intuitions

## 8. Planner Integration: Three Levels

This is where smelt functions differ most fundamentally from Jinja macros. In dbt, macros are expanded to text before anything sees them. In smelt, functions are **visible to the planner as first-class nodes** with typed interfaces and declared properties.

### Why Not Just Expand?

If functions were expanded to plain SQL before the planner runs, the planner loses semantic information. It sees `SUM(CASE WHEN ts - LAG(ts) OVER (...) > INTERVAL '30 minutes' THEN 1 ELSE 0 END) OVER (...)` instead of knowing "this is a session rollup." Pattern-matching on raw SQL to rediscover this structure is fragile and will break with any variation.

Keeping functions in the IR means:
- Planner rules match on **function names and properties**, not SQL patterns
- The function's **type contract** (return schema) serves as a safety check for rewrites
- **Column provenance** through function boundaries is explicit, not inferred from SQL

### Three Rule Levels

The planner operates at three distinct levels, each with different visibility into functions:

#### Level 1: Logical → Logical (pre-expansion)

Rules rewrite the logical DAG. Functions are **nodes with rich typed interfaces** — planner rules match on function names, parameter types, declared properties, and **compiler-derived structural metadata**, not on raw SQL.

The compiler analyzes function bodies and attaches structural metadata to the function's type:
- **Column provenance map:** Which output columns come from which input tables/parameters
- **Join graph:** Which tables are joined, join type (LEFT/INNER), and cardinality (1:1, 1:N)
- **Declared properties:** `@deterministic`, `@idempotent`, `@append_only`, etc.

This metadata is derived automatically — the function author writes plain SQL and the compiler extracts the structure. Authors can also add explicit property annotations for semantics the compiler cannot infer. In PLT terms, this is a **refinement type**: the type carries structural invariants beyond just the fragment sort.

**What happens here:**
- **Predicate pushdown into blocks.** A downstream WHERE clause is pushed into a function's `filters` parameter.
- **Fusion.** Two adjacent function calls are merged into a single, more efficient function.
- **Join elimination.** Unused output columns are traced through the provenance map; if all columns from a 1:1 LEFT JOIN are unused, the join is removed (see Example 3 below).
- **Semantic validation.** A function marked `@requires_append_only` is checked against its source.

Planner rules reason about the typed interface and its metadata — they don't pattern-match on the function's SQL body. The function body is only visible to the compiler (for metadata extraction) and to Level 2 (for expansion).

#### Level 2: Logical → Physical (strategy selection and expansion)

Rules choose an execution strategy and expand functions into strategy-specific SQL. The expansion is not mechanical inlining — it's **guided by the strategy.**

**A function might expand differently depending on the strategy:**
- **Full rebuild:** Expand body as-is.
- **Incremental append:** Expand with a temporal filter injected into the source scan.
- **Incremental merge:** Expand with affected-key detection and recomputation.

The function author writes the pure logical version. Planner rules produce strategy-specific expansions. The function's declared return type is the safety contract: the rule must produce the same output schema.

#### Level 3: Physical → Execution Plan (multi-statement orchestration)

A single physical node becomes one or more concrete SQL statements with control flow.

**Examples:**
- An incremental merge becomes: `CREATE TEMP TABLE → DELETE matching rows → INSERT FROM temp → DROP temp`
- A cross-engine model becomes: `Run query on Spark → Write Parquet → COPY INTO DuckDB`
- A validated write becomes: `Run query → Check row count / schema invariants → Swap target table`

Functions are already expanded at this level, but function **properties** still matter:
- `@idempotent` tells Level 3 that retry is safe
- `@deterministic` tells it that re-execution produces the same result

### Function Properties as Optimization Hints

Annotations on functions serve double duty — they help the type checker AND the planner:

```sql
smelt.define session_rollup(
    source: TableExpr @append_only,
    user_key: Expr<source>,
    ts_expr: Expr<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = ()
) -> TableExpr @deterministic @idempotent AS (
    ...
)
```

- `@append_only` on `source`: planner checks the source is actually append-only; enables incremental strategies
- `@deterministic` on return: planner knows re-execution is safe
- `@idempotent` on return: execution planner knows retry won't corrupt data

Some properties are verifiable by the compiler (e.g., `@deterministic` — check for `RANDOM()`, `NOW()`). Others are declared by the user and trusted. This parallels Rust's `unsafe` — the compiler checks what it can, and trusts the programmer on the rest.

## 9. Examples

### Example 1: Expression Function — safe_divide

The simplest case. Reusable across any model without modification.

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
-- models/product_metrics.sql
SELECT
    product_id,
    total_revenue,
    total_cost,
    smelt.fn.safe_divide(total_revenue - total_cost, total_revenue) AS margin_pct,
    smelt.fn.safe_divide(total_revenue, units_sold) AS revenue_per_unit
FROM smelt.ref('product_summary')
```

**What the compiler does:**
1. Verifies `total_revenue - total_cost` and `total_revenue` are numeric (they are — both are aggregated sums)
2. Expands to `CASE WHEN total_revenue = 0 OR total_revenue IS NULL THEN NULL ELSE CAST((total_revenue - total_cost) AS DOUBLE) / CAST(total_revenue AS DOUBLE) END`
3. Infers return type as `DOUBLE` (nullable, since the CASE can produce NULL)

**What the LSP does:**
- Hover on `safe_divide` shows: `(Numeric, Numeric) -> Double?`
- Go-to-definition jumps to `functions/core/safe_divide.sql`
- Completions inside the call show available columns from `product_summary`

**Testing:**
```sql
-- tests/test_safe_divide.sql
---
test: true
---
SELECT
    smelt.fn.safe_divide(10, 3) AS normal_case,
    smelt.fn.safe_divide(10, 0) AS zero_denom,
    smelt.fn.safe_divide(NULL, 5) AS null_num,
    smelt.fn.safe_divide(10, NULL) AS null_denom
-- Expected: ~3.33, NULL, NULL, NULL
```

### Example 2: Session Rollup with PASSING

A reusable model pattern: sessionize events and compute per-session metrics. The caller provides the source table, key expressions, and custom metrics.

**Definition:**
```sql
-- functions/patterns/session_rollup.sql
smelt.define sessionize(
    source: TableExpr,
    user_key: Expr<source>,
    ts_expr: Expr<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
    SELECT source.*,
        SUM(CASE WHEN ts_expr - LAG(ts_expr)
            OVER (PARTITION BY user_key ORDER BY ts_expr)
            > gap THEN 1 ELSE 0 END)
        OVER (PARTITION BY user_key ORDER BY ts_expr) AS session_id
    FROM source
)

smelt.define session_rollup(
    source: TableExpr,
    user_key: Expr<source>,
    ts_expr: Expr<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Predicate<source> = TRUE
) -> TableExpr @deterministic AS (
    WITH sessionized AS (
        smelt.fn.sessionize(source, user_key, ts_expr, gap)
    )
    SELECT
        user_key,
        session_id,
        MIN(ts_expr) AS session_start,
        MAX(ts_expr) AS session_end,
        COUNT(*) AS event_count,
        metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_key, session_id
)
```

**Usage (with PASSING):**
```sql
-- models/web_sessions.sql
---
name: web_sessions
materialization: table
incremental:
  enabled: true
  event_time_column: session_start
---
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_key => user_id,
    ts_expr => event_timestamp,
    gap => INTERVAL '20 minutes'
)
PASSING metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages,
    smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event
)
PASSING filters AS (
    event_type != 'bot'
    AND user_id IS NOT NULL
)
```

**What the planner does:**

*Level 1 (Logical → Logical):* The planner sees a `session_rollup` node. It checks that `web_events` is append-only (from source metadata) and that `event_timestamp` is a timestamp column. Both checks pass.

*Level 2 (Logical → Physical):* A planner rule selects the incremental-append strategy. It expands `session_rollup` with a temporal filter: `FROM web_events WHERE event_timestamp > :watermark` instead of `FROM web_events`. The expanded SQL still matches the declared return schema — safe.

*Level 3 (Physical → Execution):* The incremental strategy produces:
1. `CREATE TEMP TABLE __staging AS (SELECT ... WHERE event_timestamp > :last_watermark)`
2. `DELETE FROM web_sessions WHERE session_id IN (SELECT session_id FROM __staging)` — handles late events extending existing sessions
3. `INSERT INTO web_sessions SELECT * FROM __staging`
4. Update watermark

**Note how the function properties flow through all three levels:** `@deterministic` (declared on `session_rollup`) tells Level 3 that replaying a failed batch is safe. `@append_only` (required on `source`) tells Level 2 that incremental processing is valid.

### Example 3: Join Elimination via Function-Aware Planning

This example demonstrates why planner-visible functions enable optimizations that blind expansion cannot. It is based on a real pattern from the retail_analytics example.

**Setup:** A reusable enrichment function joins a fact table to multiple dimension tables:

```sql
-- functions/enrichment/enrich_order.sql
smelt.define enrich_order(
    source: TableExpr,
    customer_id_expr: Expr<source, Integer>,
    product_id_expr: Expr<source, Integer>
) -> TableExpr AS (
    SELECT
        source.*,
        c.segment AS customer_segment,
        c.country AS customer_country,
        c.name AS customer_name,
        p.category AS product_category,
        p.brand AS product_brand,
        p.department AS product_department
    FROM source
    LEFT JOIN smelt.ref('dim_customers') c
        ON customer_id_expr = c.customer_id
    LEFT JOIN smelt.ref('dim_products') p
        ON product_id_expr = p.product_id
)
```

The function enriches an order table by joining customer and product dimensions. Both are LEFT JOINs, so they never filter rows from the source. Both dimension tables have unique primary keys (`customer_id`, `product_id`), so each join matches at most one row — 1:1 cardinality.

**Model A — Uses both dimensions:**
```sql
-- models/order_analysis.sql
SELECT
    customer_segment,
    product_category,
    SUM(amount) AS total_revenue
FROM smelt.fn.enrich_order(
    source => smelt.ref('orders'),
    customer_id_expr => customer_id,
    product_id_expr => product_id
)
GROUP BY customer_segment, product_category
```

Both joins are needed. No optimization possible.

**Model B — Uses only customer columns:**
```sql
-- models/customer_revenue.sql
SELECT
    customer_segment,
    customer_country,
    SUM(amount) AS total_revenue,
    COUNT(*) AS order_count
FROM smelt.fn.enrich_order(
    source => smelt.ref('orders'),
    customer_id_expr => customer_id,
    product_id_expr => product_id
)
GROUP BY customer_segment, customer_country
```

This model uses `customer_segment` and `customer_country` (from `dim_customers`) but **no columns from `dim_products`**. The `dim_products` JOIN is dead code.

**Level 1 planner rule — Join elimination:**

```
rule eliminate_unused_1to1_left_join:
    match: function F containing LEFT JOIN to table T
    when:
        - no column from T is used by any downstream consumer of F
        - T's join key is declared unique (primary key or unique constraint)
    then:
        rewrite F to remove the JOIN to T
```

The planner can apply this rule because:
1. **Column provenance is explicit.** The function's typed interface tells the planner that `customer_segment` comes from `dim_customers` and `product_category` comes from `dim_products`. No need to trace lineage through raw SQL.
2. **Join cardinality is known.** The LEFT JOIN with a unique key means at most 1:1 — removing the join doesn't change row count.
3. **Downstream column usage is known.** The typed logical CST tells the planner exactly which columns `customer_revenue` consumes.

**After optimization, Model B effectively becomes:**
```sql
SELECT
    c.segment AS customer_segment,
    c.country AS customer_country,
    SUM(orders.amount) AS total_revenue,
    COUNT(*) AS order_count
FROM orders
LEFT JOIN dim_customers c ON orders.customer_id = c.customer_id
GROUP BY c.segment, c.country
```

The `dim_products` join has been eliminated entirely — fewer tables scanned, simpler query plan, faster execution.

**Why this requires planner-visible functions:**

With blind expansion, the planner would need to:
1. Expand the function to raw SQL
2. Pattern-match the SQL to identify individual JOINs
3. Trace column lineage through the expanded SQL to determine which columns come from which JOIN
4. Check uniqueness constraints on each joined table
5. Determine which columns are consumed downstream

Steps 2-3 are fragile and will break with SQL variations (subqueries, CTEs, aliasing). With function-aware planning, the planner matches on the function's declared structure and reads column provenance directly from the typed interface.

**Broader applicability:** This optimization isn't limited to user-defined functions. It applies wherever smelt can see a join whose columns aren't consumed downstream. Functions make it practical because they formalize the pattern with explicit types.

## 10. Comparison to Existing Systems

| System | Approach | Lesson for smelt |
|--------|----------|-----------------|
| **dbt Jinja macros** | Untyped text substitution | Solves the reuse problem but destroys analyzability. smelt functions must be the opposite — typed composition that preserves analysis. |
| **Rust `macro_rules!`** | Hygienic, fragment-sorted (`$x:expr`, `$t:ty`, `$s:stmt`) | Proof that fragment sorts work at scale. Rust's `expr` is our `Expr<T>`, `stmt` is our `TableExpr`. Hygiene = lexical scoping. |
| **Dhall** | Total, typed, no side effects, import-based composition | Totality from no-recursion. Modest type system covers 95% of cases. Don't need dependent types or type-level programming. |
| **C++ templates** | Untyped expansion, check at instantiation | Good error tracing can compensate for late checking (Tier 1 annotations). But smelt should also support early checking (Tier 2-3). |
| **TypeScript** | Gradual typing (strict optional, `any` escape hatch) | Adoption story — start untyped, add types as code matures. Same trajectory for smelt function annotations. |
| **Malloy** | First-class dimensions and measures with reusable definitions | Deep semantic modeling. smelt's approach is less opinionated — SQL fragments rather than a new semantic layer. More migration-friendly. |
| **PRQL** | Functions as first-class values, pipeline syntax | Functions over expressions work well. But PRQL is a whole new language; smelt extends SQL. |
| **Terra** | Staged programming — generate low-level code from high-level | "Generate SQL from a composition language" is exactly this framing. smelt's staging is simpler (one stage of expansion). |

### The Dhall Connection

Dhall is particularly relevant because it demonstrates that a **non-Turing-complete language with a modest type system** can replace complex templating in practice. Dhall replaced YAML/JSON templating (Kubernetes configs, CI pipelines) the same way smelt functions would replace Jinja SQL templating.

Key Dhall lessons:
- **Totality** (guaranteed termination) comes for free from no-recursion — smelt gets this automatically.
- **Records and unions** cover most use cases — smelt's SQL types (numeric, string, timestamp) plus fragment sorts (Expr, TableExpr, SelectItems) are the equivalent.
- **Imports** are the composition mechanism — smelt's `smelt.fn.*` namespace serves this role.
- **You don't need fancy types.** Dhall has no dependent types, no type-level programming, no HKTs. Its type system is modest but sufficient. smelt should follow this example.

## 11. Limits of the Design

Even at maximum ambition, some things remain outside the design:

- **Dynamic schema construction.** You cannot write a function that takes column names as runtime strings and produces a SELECT with those columns. The set of columns must be known at compile time. (`SelectItems` parameters cover the "list of things" case without requiring variadics.)
- **Conditional structure.** A function cannot return a JOIN sometimes and a subquery other times based on a runtime value. SQL structure is fixed at compile time. (Conditional *expressions* like CASE/WHEN are fine.)
- **Runtime parameterization of sorts.** Fragment type parameters are compile-time. You can pass runtime values as `Expr` (e.g., `WHERE col > smelt.param('cutoff')`), but you can't choose between different SQL structures at runtime.
- **Recursive patterns.** No function can call itself. Recursive CTEs remain a SQL-level feature, not a function-level one.

These limitations are deliberate. The Jinja use cases that hit them are exactly the ones that produce unmaintainable code.

## 12. Implementation Decisions

### Planner metadata: explicit annotations first (decided April 14, 2026)

Refinement type metadata (column provenance maps, join graphs) will be **explicitly annotated** by function authors rather than automatically derived by the compiler. Authors declare structural properties like `@joins(dim_customers LEFT 1:1)` and `@provenance(customer_segment -> dim_customers.segment)`.

**Rationale:** Automatic derivation requires a full lineage analyzer — a substantial compiler component. Explicit annotations let the planner integration ship without this, while keeping the door open to add automatic derivation later as a pure DX improvement (no semantic changes needed). In practice, only "model template" functions (session_rollup, enrich_order) benefit from planner-level optimization, so the annotation burden is concentrated on a small number of high-value functions.

### CTE context bindings: include in initial design (decided April 14, 2026)

CTE-derived context bindings (e.g., `SelectItems<Agg, sessionized>` where `sessionized` is a `WITH` clause in the function body) are worth the implementation investment and should not be deferred.

**Rationale:** Ambiguous column references are one of the most common and frustrating errors in SQL development. CTE context bindings catch "column X doesn't exist in Y" at the call site before expansion — a qualitative improvement over errors that surface in generated SQL after expansion. The implementation cost is also less than it appears: computing the output schema of a CTE is the same schema inference the type system already performs for models and function calls. The incremental work is wiring CTE schema computation into the context binding checker, not building a new analysis from scratch.

### Default values: self-contained, type-checked, omission-as-empty (decided April 15, 2026)

Default values for parameters follow these rules:

- **Self-contained.** A default expression cannot reference other parameters. This keeps default evaluation order trivial and avoids a mini-scope-resolution problem at the signature layer.
- **Type-checked at definition time.** A default must satisfy the parameter's declared type. This is a free Tier 2 check — defaults are part of the signature, so checking them is part of checking the signature.
- **Omission means "splice nothing"** for fragment-typed parameters. A `SelectItems` parameter with a default of "no items" is expressed by simply omitting the argument at the call site — no `()` empty-list syntax needed in the source. `Predicate` parameters that should default to "no filter" use `= TRUE` (a real SQL literal). This avoids inventing non-SQL syntax for empty fragments.

### No variadic parameters in v1 (decided April 15, 2026)

Variadic / splat parameters are out of scope for v1. The `SelectItems` fragment sort already covers the "pass a list of things" case ergonomically. Variadics can be added later without breaking existing functions.

### Function ordering and cycle detection (decided April 15, 2026)

- **No intra-file ordering requirement.** A function may call any other function defined anywhere in the project, regardless of file or position within a file. Forward references are first-class.
- **Cycle detection lives in `smelt-db`** as a Salsa query over the function-call graph. Detection runs against function definitions only — model-to-function and function-to-model edges are not part of the cycle check.

### Visibility: all functions public in v1 (decided April 15, 2026)

There is no `pub` / `private` distinction. Every function defined in the project is callable from any model or other function. Adding visibility modifiers later is non-breaking (default stays public).

### Function testing deferred (decided April 15, 2026)

A dedicated test mechanism for functions (e.g., `test: true` files calling `smelt.fn.*` directly) is not part of v1. Functions remain testable indirectly through models that use them. A first-class function-test workflow is a follow-up.

### LSP signature stability under broken bodies (decided April 15, 2026)

When a function body becomes invalid mid-edit:

- **Tier 2 / Tier 3 functions** retain their signature. Call sites continue to type-check against the declared parameter and return types. The error is contained to the function being edited; downstream models do not cascade red.
- **Tier 1 functions** have no signature independent of the body, so call sites cannot be checked while the body is broken. This is unavoidable, and is one more reason to encourage Tier 2+ for shared functions.

This asymmetry should be surfaced in LSP UX (e.g., a hint on Tier 1 functions: "annotate parameters to prevent cascading errors during edits").

### Namespacing: directory-derived (decided April 15, 2026)

Function paths under `smelt.fn.*` mirror the directory layout under `functions/`. `functions/patterns/session_rollup.sql` defines `smelt.fn.patterns.session_rollup`. This matches the `models/` convention and avoids inventing a separate namespace declaration.

### No overloading in v1 (decided April 15, 2026)

Function names are unique within their namespace. Two functions named `margin` with different parameter types are not allowed. Overloading combined with gradual typing is a known footgun (resolution rules become annotation-tier-dependent), and the cost of adding overloading later is low.

### Window expressions as a distinct sort with linear subtyping (decided April 18, 2026)

Window functions (`SUM(x) OVER (...)`, `ROW_NUMBER() OVER (...)`) are represented as a distinct fragment sort `WindowExpr<T>`, not as a modifier on `AggExpr<T>` or a hidden flag on `Expr<T>`. The three expression sorts form a linear subtyping chain: `Expr<T> <: AggExpr<T> <: WindowExpr<T>`, matching SQL's evaluation order (WHERE → GROUP BY → Window → SELECT).

**Rationale:** Window expressions have placement rules distinct from both scalars and aggregates — they cannot appear in WHERE or HAVING, cannot nest inside other window functions, and cannot nest inside aggregates. Making this a distinct sort enforces these constraints through the type system rather than as ad-hoc checks. The linear subtyping chain avoids the need for union types: a function author picks a single level on the chain, and subtyping allows callers to pass any expression at that level or below.

**Alternatives considered:**
- **Hidden flag on `Expr<T>`** — Defeats the purpose of the fragment sort system (explicitness).
- **`OVER` as a modifier producing `Expr<T>`** — Loses the placement restrictions; a windowed result would appear to be valid in WHERE.
- **Separate sort with union types** — Unnecessary complexity since the sorts form a linear chain, not a diamond.

### Functions are additive (decided April 15, 2026)

Introducing functions does not change the meaning of existing models. Models that don't call any function compile and run identically to today. The `smelt.define` and `smelt.fn.*` syntax is purely additive to the grammar.

## 13. Open Questions

The decisions in §12 and §16 close most of the prior open questions. Items marked ✅ have been resolved.

### ✅ Block syntax surface (resolved April 18, 2026 → §16 decision 3)

**Decision:** `PASSING name AS (...)` clauses. Uses a distinct keyword with SQL/XML precedent, avoiding CTE ambiguity. Parenthesized content, no whitespace sensitivity. See §5 for updated syntax.

### ✅ Grammar for `smelt.define` (resolved April 18, 2026 → §16 decision 5)

**Decision:** Multiple `smelt.define` per file (consistent with models). Function files are definition-only (no SELECT statements). No frontmatter for v1. Namespacing is directory-derived per §12.

### Annotation syntax (must decide before plan)

`@deterministic`, `@append_only`, `@joins(dim_customers LEFT 1:1)`, `@provenance(...)` are used throughout the paper but never given a formal grammar. Need to decide:

- Where annotations may attach (parameter, return type, whole function)
- The grammar for structured annotation arguments
- Which annotations ship in v1 vs. are reserved

### ✅ `Column<T>` parameters: bare refs only, or expressions? (resolved April 18, 2026 → §16 decision 2)

**Decision:** `Column<T>` dropped entirely for v1. Use `Expr<T>` with context bindings everywhere. A bare column reference is a trivial expression. `Column<T>` can be reintroduced as a subtype if lineage/planner needs require it.

### MVP scope (to be decided in implementation planning)

- Which fragment sorts ship in v1?
- Which annotation tier ships first?
- Does v1 include any Level 1 planner rule, or pure expansion only?
- Does v1 include the PASSING syntax, or are expression-only functions enough to validate the architecture?

Note: these scoping questions will be decided during the separate implementation planning process, not in this research document. The research doc should document dependencies between features and what each teaches us.

### ✅ Relationship to `smelt.metric()` (resolved April 18, 2026 → §16 decision 6)

**Decision:** Out of scope. `smelt.metric()` doesn't work today. Functions and metrics are independent; can coexist later. No parser unification needed.

### ✅ Specification tightening (resolved April 18, 2026 → §16 decision 4)

- **Union context disambiguation** — Resolved by replacing union contexts with the no-overlap rule + `smelt.as_struct()` namespacing. No ambiguous column names possible in a parameter's context.
- **CTE context checking boundary** — Resolved by inference-based context resolution. Structural checks (correct sort, correct position) happen at definition time. Column-name validation happens at the call site where schemas are known. Context annotations in signatures are optional documentation that gets validated against inference.

### Already deferred / not blocking

- **LSP block-context completion** — architecturally hard, can land after basic diagnostics.
- **Multiple expansion modes per author** — committed to "the planner's job" unless pain emerges.
- **Function tests** — deferred per §12; functions remain testable through models that use them.
- **Package ecosystem / registry** — not v1.
- **Python model interaction** — functions are SQL-only; Python models are opaque table producers reachable via `smelt.ref()`.

## 13. Summary

smelt functions are **typed, composable SQL fragments** that replace Jinja macros while preserving static analysis:

- **SQL fragment types** ensure structural well-formedness
- **Lexical scoping** makes functions self-contained and analyzable
- **Optional annotations** allow gradual adoption of type rigor
- **Block syntax** provides ergonomic multi-line fragment passing
- **Three-level planner integration** makes functions visible to optimization
- **No recursion** guarantees termination

The design occupies a specific point in the design space: more powerful than simple expression macros, less complex than a full functional programming language. It covers the three categories of Jinja usage that smelt currently lacks (expression reuse, fragment generation, model templates) while maintaining the guarantees that make smelt valuable (type checking, planner visibility, LSP support, testability).

The phased implementation path — Tier 1 annotations first, then Tier 2 and 3 — allows shipping incremental value while building toward the full design.

## 14. PL/Compiler Expert Review

**Date:** April 14, 2026
**Reviewer:** Claude (prompted as PL/compiler expert)
**Revision:** Updated April 14, 2026 after paper revisions addressing original review points 1, 2, 4, and 6. Context bindings now support CTE references and union contexts. Scoping is honestly described as hybrid. Planner model uses refinement types. Error message contract is specified. Points 3 (block syntax complexity) and 5 (Malloy comparison) remain as open considerations.

### PLT Techniques Being Used (Named)

1. **Fragment sorts / syntactic categories** (Section 3) — This is the Rust `macro_rules!` technique of *fragment specifiers* (`$e:expr`, `$s:stmt`), which itself derives from the PL concept of **syntactic sorts** in multi-sorted algebras. The idea: a macro system that knows the grammar can restrict substitution to structurally valid positions. MetaML, Template Haskell, and Rust all do this. It's the single strongest idea in the paper.

2. **Staged metaprogramming** (Section 8) — Functions that "compile away" are a one-stage version of **multi-stage programming** (Taha & Sheard, MetaML). The paper correctly identifies Terra as the closest analog. The three-level planner integration is essentially a **multi-pass lowering pipeline** — the same architecture as MLIR (Multi-Level IR), where each dialect level carries different semantic information and rewrites happen at the appropriate level.

3. **Hygienic macro expansion with structural column resolution** (Section 7) — The revised scoping model correctly identifies two layers: parameter bindings are **hygienic** (Kohlbecker et al., 1986), while bare column names against `TableExpr` parameters use **structural resolution** — closer to **row polymorphism** (OCaml object types, PureScript row types) than pure lexical scoping. The paper now names this honestly as a hybrid, which is the right call. See remaining concerns below.

4. **Gradual typing with error contracts** (Section 6) — The three-tier annotation model is textbook **gradual typing** (Siek & Taha, 2006), following the TypeScript adoption curve. The addition of explicit error message contracts per tier is a significant improvement — this is where gradual typing systems succeed or fail in practice, and the paper now commits to specific error quality guarantees rather than leaving them implicit.

5. **Totality via structural restriction** (no recursion) — This is the Dhall approach, which itself comes from **total functional programming** (Turner, 2004). Banning recursion guarantees termination trivially. The paper correctly notes this is sufficient.

6. **Refinement types for SQL functions** (Section 8) — The revised planner model explicitly adopts **refinement types**: function types carry compiler-derived structural metadata (column provenance maps, join graphs, declared properties) beyond the basic fragment sort. This resolves the original tension between "opaque nodes" and the planner needing internal structure. The paper now correctly names this as refinement typing, following the pattern from liquid types (Rondon et al., 2008) and similar systems.

7. **Context-dependent type binding** (Section 3) — The expanded context binding system (`Column<source, T>`, `Predicate<source>`, `SelectItems<Agg, sessionized>`, union contexts) is a restricted form of **dependent types** where a type parameter references a value-level binding. The extension to CTE-derived contexts and union contexts moves this closer to a proper **row-polymorphic system with structural subtyping** — the union `source | customers | products` is essentially a join of row types.

### Critical Analysis

#### What works well

**The fragment sort system is the right core idea.** Jinja's fundamental failure is operating on strings. The moment you introduce syntactic sorts, you can statically prevent nonsense compositions. Rust's macro system proved this works at industrial scale — the key lesson being that you need *enough* sorts to be useful but not so many that the system becomes its own type theory.

**The context binding system is well-designed.** The revised Section 3 addresses the original concern about `Column<source>` by generalizing context bindings across all fragment sorts. The key insight — that `metrics` binds to `sessionized` while `filters` binds to `source`, giving the author control over what each caller-provided fragment can see — is a powerful scoping mechanism. This is essentially **capability-based access control** applied to SQL column namespaces: the function author grants each parameter access to specific table contexts.

The CTE context mechanism (where `sessionized` is a `WITH` clause in the body, and the compiler derives its schema) is elegant. It means the author can expose *computed* contexts — not just the raw input tables — to callers. The coupling between signature and body (changing a CTE's SELECT list changes what callers can reference) is a real trade-off, but the paper correctly identifies it as intentional: the context binding names the splice-point scope.

**The union context design handles joins naturally.** `SelectItems<source | customers | products>` for multi-table contexts follows SQL's own resolution rules (ambiguous names require qualification). This avoids inventing new semantics for a well-understood problem.

**Late expansion with refinement types is architecturally sound.** The revised Level 1 planner model — functions as nodes with rich typed interfaces including compiler-derived structural metadata — resolves the original contradiction between "opaque nodes" and needing join graphs for optimization. The join elimination example (Section 9, Example 3) is now consistent with the planner model: the planner reads column provenance from the refinement type, not by inspecting the SQL body.

**The gradual annotation strategy now includes error contracts.** The error message contract (Section 6) is a critical addition. The Tier 1 contract — "show the call site with parameter mapping, not just a raw SQL error" — is exactly the minimum viable error experience. The progression from traced bindings (Tier 1) to declared contracts (Tier 2) to author-isolated errors (Tier 3) is clean. The "author complexity, user clarity" principle provides a coherent narrative for why this progression matters.

#### Where to push back

**1. Context binding scope resolution needs more specification.**

The context binding system is well-conceived but leaves edge cases underspecified. Consider:

```sql
smelt.define foo(
    a: TableExpr,
    b: TableExpr,
    col: Column<a | b, Integer>  -- union context across parameters
)
```

What happens when `a` and `b` both have a column named `id` of type `Integer`? The paper says "ambiguous column names require qualification — the same rule as standard SQL." But in standard SQL, qualification uses table aliases (`a.id`, `b.id`). In the context binding system, the "table" is a parameter name. Does the caller write `a.id` or does the compiler require the caller to disambiguate some other way?

Similarly, for CTE-derived contexts: if a CTE `sessionized` selects `source.*` plus `session_id`, and `source` is a `TableExpr` parameter, the schema of `sessionized` depends on the call site. This means **context-bound type checking is call-site-dependent even for Tier 2+** — the compiler can check that the *form* of the caller's fragment is correct (it references columns, not arbitrary expressions), but validating specific column names requires knowing the call-site schema. This is fine, but it's worth being explicit about: CTE context bindings give you *structural* guarantees (the fragment references columns from the right table) but column-name validation is still call-site-dependent.

**Recommendation:** Specify the disambiguation rules for union contexts (especially parameter-parameter unions vs. parameter-CTE unions), and clarify which checks happen in isolation vs. at the call site for CTE-derived contexts.

**2. The block syntax introduces a second grammar.**

The `{ metrics: ... filters: ... }` block syntax is effectively a **domain-specific sub-language** embedded in the call site. This has worked before (Ruby blocks, Kotlin DSL builders, Groovy closures in Gradle), but each time it creates a parsing and tooling burden disproportionate to the syntactic convenience.

Specific concerns:
- What's the delimiter between sections? Newlines? Commas? The examples use blank lines, which makes the parser whitespace-sensitive in a context where SQL is not.
- How does error recovery work inside blocks? You now have nested parsing contexts (SQL -> function call -> block -> SQL inside block).
- The LSP complexity note in Section 12 is an understatement — you need *contextual* completion that depends on partial expansion, which is one of the hardest LSP problems.

**Compare with:** Kotlin's trailing lambda syntax, which has the same "pass a block to a function" ergonomic goal but avoids introducing named sections. Consider whether named parameters with parenthesized SQL fragments (the "ugly" version) plus good formatter support might be 80% of the value at 20% of the parser complexity.

**3. The comparison table undersells Malloy.**

Malloy isn't just "deep semantic modeling" — it's the closest direct competitor to this design. Malloy's `dimension` and `measure` declarations are essentially `Expr<T>` and `AggExpr<T>` with fixed scoping. Malloy's `source` extensions are `TableExpr` transformers. The key difference is that Malloy chose a new query language while smelt extends SQL, but the *function composition model* is very similar. A fair comparison would help readers understand the actual trade-off: Malloy gets cleaner semantics by abandoning SQL syntax; smelt gets migration compatibility by extending SQL syntax but inherits SQL's scoping messiness.

**4. Refinement type metadata derivation is implementation-heavy.** *(Resolved — see Section 12)*

The revised planner model is conceptually cleaner, but the compiler work to *derive* structural metadata automatically is substantial. Extracting column provenance maps and join graphs from arbitrary SQL bodies means the compiler must essentially build a relational algebra representation of each function body — this is a query optimizer's job.

**Decision:** Start with explicit annotations (`@joins`, `@provenance`) and add automatic derivation later as a DX improvement. This lets the planner integration ship without requiring a full lineage analyzer, while keeping the design compatible with automatic derivation in the future.

### Historical Precedents Worth Studying

- **SML Functors / OCaml modules** — Parameterized modules that produce types based on input types. `Column<source, T>` is a simplified version of this: a type that depends on a module's signature.
- **Scala's path-dependent types** — `source.Column` where the type depends on a specific value. The context binding system is more constrained (contexts are named table scopes, not arbitrary paths), which avoids Scala's complexity pitfalls while preserving the useful dependent relationship.
- **Template Haskell** — Multi-stage compilation where generated code is type-checked after splicing. The Tier 1 strategy is exactly this. TH showed it works but that error messages are the main usability challenge — the new error message contract directly addresses this lesson.
- **MLIR's progressive lowering** — The three-level planner is structurally identical to MLIR's dialect-to-dialect lowering. MLIR's lesson: you need clear contracts at each level boundary, which maps to the paper's "return type as safety contract."
- **Liquid types (Rondon et al., 2008)** — Refinement types that carry logical predicates beyond the base type. The compiler-derived structural metadata (join graphs, provenance maps) attached to function types is a domain-specific form of refinement typing, where the "predicates" are relational algebra properties rather than logical formulas.

### Summary Verdict

The revised paper is substantially stronger. The four main weaknesses from the original review have been addressed:

1. **Context bindings** (originally: "`Column<source>` is dependent typing in disguise") — Now a well-specified system covering all fragment sorts, with CTE-derived contexts and union contexts. The dependent typing is acknowledged and scoped appropriately. Edge cases around disambiguation remain (see point 1 above), but the core design is sound.

2. **Scoping** (originally: "names the hybrid honestly") — The revised Section 7 correctly describes the two-layer model: lexical parameters + structural column resolution (row polymorphism). This is honest and technically precise.

3. **Planner model** (originally: "opaque vs. refinement type contradiction") — The revised Section 8 explicitly adopts refinement types with compiler-derived metadata, resolving the contradiction. The implementation cost of metadata derivation is the remaining concern (see point 4 above).

4. **Error messages** (originally: "missing error message design") — The error message contract in Section 6 commits to specific quality guarantees per tier, with the "author complexity, user clarity" principle providing coherent motivation.

**Remaining open items** are block syntax complexity (point 2) and the Malloy comparison (point 3) — both are secondary to the core type system design. Refinement type metadata derivation (point 4) has been resolved: explicit annotations first, automatic derivation later.

**Implementation path is now clearer.** Two key decisions reduce the risk: explicit planner annotations avoid the need for a lineage analyzer at launch, while CTE context bindings are confirmed as worth the investment (leveraging existing schema inference). The remaining implementation sequencing question is how Tier 1 error tracing, context binding checking, and the planner annotation system interact — a phased implementation plan identifying which pieces can ship independently would strengthen the path from paper to product.

## 15. Typing Built-in SQL Functions

### Motivation

The fragment sort system in §3 was designed for user-defined `smelt.define` functions, but every model also calls built-ins: `COALESCE`, `SUM`, `CAST`, `SUBSTRING`, `generate_series`, and so on. Typical models call built-ins far more often than user functions. If built-ins carry the same fragment-typed signatures, every SQL call gets the same compile-time checking, hover types, and completion that `smelt.fn.*` gets. Planner rules can also match on built-in names the same way they match on user functions.

This is probably the highest-leverage extension of the type system. It is worth asking up front which built-ins fit, which need modest extensions, and which are fundamentally outside scope.

### What fits with no new machinery

Many built-ins map directly onto the existing sorts:

| Built-in shape | Signature |
|----------------|-----------|
| Pure scalar (`LOWER`, `ABS`, `LENGTH`) | `Expr<T1> -> Expr<T2>` |
| Binary scalar (`POWER`, `MOD`) | `(Expr<T>, Expr<T>) -> Expr<T>` |
| Aggregates (`SUM`, `COUNT`, `AVG`) | `Expr<T> -> AggExpr<T>` |
| Predicate-producing (`IS NULL`, `LIKE`) | `Expr<T> -> Predicate` |
| Simple table functions (`generate_series(1, 10)`) | `(Expr<Int>, Expr<Int>) -> TableExpr` |

These are the majority of the SQL standard library. No new typing machinery required.

### What needs extensions

Several ubiquitous built-ins expose gaps:

**1. Generics / type parameters.** `COALESCE(a, b, c)` returns the common supertype of its arguments. `MAX(x)` returns the same type as `x`. `ARRAY_AGG(x)` returns `Array<T>`. Typing these requires type parameters on signatures (e.g., `COALESCE<T>: Expr<T>... -> Expr<T>`) — a modest extension paralleling Rust generics, but it introduces a type-inference step we currently don't have.

**2. Variadics.** `COALESCE`, `CONCAT`, `GREATEST`, `LEAST` all accept arbitrary arity. v1 deliberately excluded variadic *user* functions (§12). For built-ins, either we give them a privileged native-variadic form that user functions can't use, or we reintroduce variadics for both. Worth a deliberate decision.

**3. Types as arguments.** `CAST(x AS INTEGER)` passes a type where a parameter normally sits. Same shape in `TRY_CAST`, `EXTRACT(YEAR FROM ts)` (a field keyword), and parameterised types like `DECIMAL(10, 2)`. Not expressible as `Expr<T>` parameters. Options: add a dedicated `Type` (and maybe `Field`) parameter sort; or treat this syntax as primitive grammar that the checker handles specially.

**4. Keyword-argument syntax.** `TRIM(BOTH ' ' FROM x)`, `SUBSTRING(s FROM 1 FOR 3)`, `POSITION(sub IN str)`, `LISTAGG(x, ',') WITHIN GROUP (ORDER BY y)`. The SQL standard spells several built-ins with mandatory keywords rather than commas. Our `=>` named-argument syntax doesn't map onto these; they'd need to be treated as primitive grammar rather than ordinary calls.

**5. Modifier clauses on aggregates.** `SUM(x) FILTER (WHERE cond)` and `string_agg(x, ',' ORDER BY y)` attach `Predicate` / `OrderSpec` fragments to an aggregate call — which the type system already knows how to describe — but the attachment is a syntactic suffix, not a parameter. A refined `AggExpr<T>` that carries optional `filter: Predicate` and `order: OrderSpec` slots would make this explicit. Window `OVER (...)` clauses are handled by the `WindowExpr<T>` sort (§3): applying `OVER` to an `AggExpr<T>` produces a `WindowExpr<T>`, following the expression level subtyping chain.

**6. Schema-returning table functions.** `UNNEST(array_col)` produces a table whose schema depends on the array element type — typeable with generics (`UNNEST<T>: Expr<Array<T>> -> TableExpr{value: T}`). `read_csv` / `read_parquet` with auto-schema detection is **not** compile-time typeable by design: the schema is discovered at runtime. These must either accept a user-supplied schema annotation or fall back to an opaque `TableExpr` where column references are checked at expansion time.

### What is fundamentally untypeable

A small set of features cannot be fit without abandoning compile-time guarantees:

- **Auto-schema built-ins** without a schema hint (`read_csv('x.csv')`). The schema exists only after reading the file. Models using them must either carry an explicit schema annotation or accept opaque `TableExpr`.
- **Dynamic `EXECUTE` / string-templated SQL.** Out of scope — and rare in analytics SQL.
- **Untyped JSON navigation** (`col->>'foo'`). Typeable only by committing to `Text` unconditionally and requiring explicit casts, which matches most engines' existing behaviour.

### Where this lands

The direct implication for v1: the existing design already types roughly 80% of common built-ins without new machinery. The remaining 20% — generics, variadics, type-arguments, keyword syntax, modifier clauses — is a substantial but bounded extension. None of it invalidates the existing design; all of it can be added incrementally.

**Honest positioning:** the fragment sort system is a plausible foundation for typing built-ins, but built-in coverage is a separate work item with its own design pressure. A v1 that types only user functions (and leaves built-ins to the existing type inference) is still a large improvement. A v2 that extends coverage to built-ins unlocks a second, larger improvement — but it is not free.

There is also a crossover with the §6 error contract: Tier 1 call-site errors may surface through built-in calls whose arguments we can't check early. That is acceptable (it matches today's behaviour) but worth naming as a constraint the user will see.

### Open question for the v2 discussion

If we extend the type system to built-ins, do we do it through a **signature registry** (a table of built-in signatures the checker consults, one per dialect) or by making the checker aware of a small set of **primitive built-in shapes** (CAST-shaped, EXTRACT-shaped, aggregate-with-modifiers-shaped) that it handles specially? The registry approach scales with engines; the primitive-shapes approach keeps the checker simple but requires per-engine code for anything unusual. Both are viable; the choice affects how much work it is to add a new backend.

## 16. Design Decisions (April 18, 2026)

The following decisions were made in a design discussion focused on resolving ambiguities identified in §13 and stress-testing the type system's interaction with SQL scoping. These decisions simplify the design compared to earlier sections of this paper — §3, §5, and §7 have been updated with revision notes pointing here.

### Decision 1: Column Resolution Rule

**Problem:** In the hybrid scoping model (§7), it was unclear how bare names resolve when a parameter name could also be a column in a `TableExpr` parameter's schema. The function's behavior should not change based on what columns happen to exist in the source table.

**Decision: Parameters-first resolution with standard SQL fallback.**

Resolution order in function bodies:
1. **Parameter names first.** If a bare name matches a parameter, it resolves to the parameter (lexical scope wins).
2. **Standard SQL FROM-clause rules.** After parameters, bare names resolve against the FROM/JOIN/CTE scope using normal SQL rules — single table in scope allows bare names; multiple tables require qualification for ambiguous names.
3. **Shadow warning.** The compiler warns when a parameter name shadows a column in a `TableExpr` parameter's schema (detectable at the call site where the schema is known).

**In models** (regular `.sql` files, not function definitions), bare column names work as normal SQL. The parameter-first rule only applies inside `smelt.define` bodies.

**Rationale:** Parameters must take priority to preserve hygienic expansion — a function's behavior should be determined by its parameters, not by ambient column names. Standard SQL resolution for the column layer keeps functions feeling like SQL. The shadow warning catches the footgun where adding a parameter silently reinterprets a column reference. This rule is also consistent with decision 4 (no-overlap rule), which prevents ambiguous column names in parameter contexts.

### Decision 2: Drop Column\<T\> for v1

**Problem:** The `Column<T>` fragment sort was intended to distinguish bare column references from computed expressions. But no use case was found where correctness requires this distinction — a function that `PARTITION BY user_key` works identically whether `user_key` is `user_id` or `lower(user_id)`.

**Decision:** Remove `Column<T>` from the fragment sort system. Use `Expr<T>` with context bindings everywhere. A bare column reference is a trivial expression.

**Fragment sorts for v1:**

| Sort | What it represents |
|------|--------------------|
| `Expr<T>` | Scalar expression of SQL type T |
| `AggExpr<T>` | Expression containing aggregation |
| `WindowExpr<T>` | Expression containing a window function |
| `TableExpr` | Something with a schema |
| `SelectItems` | List of (expression, alias) pairs |
| `Predicate` | Boolean expression |
| `OrderSpec` | Expression + direction |

**Future:** `Column<T>` can be reintroduced as a subtype at the bottom of the expression chain (`Column<T> <: Expr<T> <: AggExpr<T> <: WindowExpr<T>`) if lineage tracking or planner optimization requires distinguishing bare column references from computed expressions.

### Decision 3: PASSING Keyword for Block Syntax

**Problem:** The original `{ metrics: ... filters: ... }` block syntax (§5) introduced a second grammar with whitespace-sensitive section delimiters. The `WITH` alternative risked confusion with CTEs. See §5 for the full alternatives analysis.

**Decision:** Use `PASSING name AS (...)` clauses after function calls.

```sql
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_key => user_id,
    ts_expr => event_timestamp
)
PASSING metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages
)
PASSING filters AS (
    event_type != 'bot' AND user_id IS NOT NULL
)
```

**Properties:**
- `PASSING` is a distinct keyword with SQL/XML precedent — zero ambiguity with `WITH` CTEs
- Parenthesized content — no whitespace sensitivity
- Attaches to the immediately preceding `smelt.fn.*` call
- Multiple `PASSING` clauses per call (one per fragment parameter)
- Nested function calls use inline named args; `PASSING` only works at the outermost call level

**Rationale:** `PASSING` avoids the CTE confusion that `WITH` would cause, while providing the same ergonomic benefit of separating multi-line SQL fragments from the function call's argument list. The SQL/XML standard uses `PASSING` for a similar purpose (passing arguments to `XMLTABLE`), providing prior art.

### Decision 4: Context Resolution — Three Rules (replaces union contexts)

**Problem:** The original union context design (`Predicate<source | customers | products>`) required a custom union type in the type system and had underspecified disambiguation rules when tables had overlapping column names.

**Decision:** Replace union contexts with three simpler rules that compose to prevent ambiguity without new type system concepts.

**Rule 1: Intersection.** A parameter's context is the intersection of columns visible at every splice point in the body. Single use = the FROM scope at that point. Multiple uses = intersection of all scopes. Empty intersection = compile error.

**Rule 2: No overlapping column names.** The context (set of columns visible to a parameter's fragment) must have unique column names. If a JOIN produces duplicate column names in a parameter's context, it is a compile error. This catches a real class of ETL bugs (ambiguous column references in joins).

**Resolution options for function authors facing overlaps:**
1. Use explicit SELECT lists in CTEs to produce clean schemas
2. Use typed `TableExpr` parameters to control the input schema (e.g., `source: TableExpr{order_id: Integer, amount: Numeric}` — `source.*` expands to exactly those columns)
3. Use `smelt.as_struct()` to namespace each table's columns

**Rule 3: `smelt.as_struct()` for namespacing.** A compile-time construct that wraps a table alias into a struct namespace:

```sql
SELECT
    source.*,
    smelt.as_struct(c EXCEPT customer_id) AS customer,
    smelt.as_struct(p EXCEPT product_id) AS product,
    extra_cols
FROM source
LEFT JOIN dim_customers c ON ...
LEFT JOIN dim_products p ON ...
```

- `customer.country` expands to `c.country` in generated SQL — zero runtime cost
- `EXCEPT` clause removes columns (typically join keys), analogous to `SELECT * EXCEPT`
- The caller references columns via struct field access: `customer.country`, `product.category`

**Context annotations** in signatures (e.g., `metrics: SelectItems<Agg, sessionized>`) are optional. The compiler infers context from splice points by default. When present, annotations serve as documentation and are validated against the inferred context. Annotations are **required** when a parameter is used in multiple different scopes (the author must specify which scope).

**Rationale:** This replaces the union context type system feature with standard SQL mechanisms. The no-overlap rule catches real bugs (ambiguous join columns). `smelt.as_struct()` solves the legitimate multi-table case using struct field access — a well-understood SQL feature. The result is a simpler type system (no union sorts) with stronger safety guarantees (no ambiguous columns possible).

### Decision 5: File Structure

**Decision:**
- Multiple `smelt.define` per file (consistent with how models work)
- Function files are definition-only (no `SELECT` statements alongside definitions)
- No frontmatter for v1
- Namespacing remains directory-derived per §12

### Decision 6: smelt.metric() Relationship

**Decision:** Out of scope. `smelt.metric()` does not work today. Functions and metrics are independent features that can coexist later. No parser unification needed for v1.

### Decision 7: MVP Scope Process

**Decision:** The research document should document dependencies between features and what each teaches us. Shipping scope and phasing will be decided in a separate implementation planning process after the design is complete. The research doc is for design clarity, not delivery planning.
