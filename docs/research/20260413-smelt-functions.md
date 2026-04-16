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
| `Expr<T>` | Scalar expression of SQL type T | SELECT, WHERE, ON, HAVING, CASE |
| `AggExpr<T>` | Expression containing aggregation | SELECT (with GROUP BY), HAVING |
| `TableExpr` | Something with a schema | FROM, JOIN, WITH |
| `SelectItems` | List of (expression, alias) pairs | SELECT clause |
| `Predicate` | Boolean expression | WHERE, ON, HAVING, QUALIFY |
| `Column<T>` | Column reference of type T | Anywhere Expr<T> is valid |
| `OrderSpec` | Expression + direction | ORDER BY |

These sorts ensure structural well-formedness: you cannot splice a `TableExpr` into a WHERE clause, or a `Predicate` into a FROM clause. The compiler checks sort-correctness at each composition point.

### Table Context Bindings

Any fragment sort that can contain column references may optionally declare which **context** its columns resolve against. A context can be:

1. **A `TableExpr` parameter** — e.g., `Column<source>` where `source` is a parameter
2. **A CTE defined in the function body** — e.g., `SelectItems<Agg, sessionized>` where `sessionized` is a `WITH` clause in the body
3. **A union of contexts** — e.g., `Predicate<source | customers>` for fragments that can reference columns from multiple tables

The compiler derives the schema of each named context: parameter schemas come from the call site, CTE schemas are computed from the function body. Context bindings are then validated against these schemas.

| Sort | With context binding | Meaning |
|------|---------------------|---------|
| `Column<T>` | `Column<source, T>` | A column from `source` of SQL type T |
| `Predicate` | `Predicate<source>` | A boolean expression whose columns come from `source` |
| `SelectItems` | `SelectItems<Agg, sessionized>` | Aggregate select items over `sessionized` columns |
| `Expr<T>` | `Expr<T, source>` | A scalar expression whose columns come from `source` |
| `OrderSpec` | `OrderSpec<enriched>` | An ordering expression over `enriched` columns |

Without a context binding, column references are resolved at expansion time (Tier 1 checking). With a context binding, the compiler validates column references at the call site against the bound context's schema — **before expansion** — producing clear, localized errors.

```sql
smelt.define session_rollup(
    source: TableExpr,
    user_col: Column<source>,                    -- column from source
    ts_col: Column<source, Timestamp>,           -- timestamp column from source
    gap: Expr<Interval> = INTERVAL '30 minutes', -- no table context (literal)
    metrics: SelectItems<Agg, sessionized> = (), -- agg over sessionized (source.* + session_id)
    filters: Predicate<source> = TRUE            -- predicate over source columns only
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

Note the deliberate asymmetry: `metrics` binds to `sessionized` (the caller can reference `session_id` in their aggregate expressions), while `filters` binds to `source` (the author restricts filtering to raw source columns only). **The author controls what each caller-provided fragment can see.** This is a meaningful design choice — narrowing the context is how authors prevent callers from depending on internal implementation details.

#### Union Contexts for Joins

When a function joins multiple tables, the author can expose a union context so callers can reference columns from any of the joined tables:

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

With `SelectItems<source | customers | products>`, the caller can pass expressions referencing columns from any of the three tables. Ambiguous column names (present in multiple contexts) require qualification — the same rule as standard SQL.

#### Design Properties

Context bindings are **always optional.** A function author who omits them gets Tier 1 behavior (check at expansion, trace errors back). An author who adds them shifts the checking earlier and gives callers better errors. This follows the **author complexity, user clarity** principle: the author bears the annotation cost; every caller benefits.

**CTE context is computed, not declared.** The author references a CTE by name in the type annotation; the compiler derives its schema from the body. This means the signature and body are coupled — changing the CTE's SELECT list may change what callers can reference. This coupling is intentional: the context binding names the *splice-point scope*, and the splice point is in the body.

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

### Calling Convention

Functions are called via the `smelt.fn.*` namespace:

```sql
-- Expression function: inline in any expression context
SELECT smelt.fn.safe_divide(revenue, cost) AS margin
FROM smelt.ref('orders')

-- Table function: used as a table source
SELECT * FROM smelt.fn.sessionize(
    source => smelt.ref('events'),
    user_col => user_id,
    ts_col => event_timestamp
)
```

### Composition

Functions can call other functions:

```sql
smelt.define session_rollup(
    source: TableExpr,
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    extra_metrics: SelectItems<Agg, sessionized> = ()
) -> TableExpr AS (
    WITH sessionized AS (
        smelt.fn.sessionize(source, user_col, ts_col, gap)
    )
    SELECT
        user_col,
        session_id,
        MIN(ts_col) AS session_start,
        MAX(ts_col) AS session_end,
        COUNT(*) AS event_count,
        extra_metrics
    FROM sessionized
    GROUP BY user_col, session_id
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
    user_col => user_id,
    ts_col => event_timestamp,
    extra_metrics => (SUM(revenue) AS total_revenue, COUNT(DISTINCT page) AS unique_pages),
    filters => (event_type != 'bot' AND user_id IS NOT NULL)
)
```

### Block Syntax as Sugar

The `{ }` block after a function call provides named sections for fragment-typed parameters:

```sql
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_col => user_id,
    ts_col => event_timestamp,
    gap => INTERVAL '20 minutes'
) {
    metrics:
        SUM(revenue) AS total_revenue,
        COUNT(DISTINCT page_url) AS unique_pages,
        smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event

    filters:
        event_type != 'bot'
        AND user_id IS NOT NULL
}
```

This desugars to passing the block contents as fragment-typed arguments. The compiler treats it identically to inline arguments. But the syntax is dramatically better for the common case of multi-line SQL fragments.

### Blocks Compose

A function can receive blocks from its caller and pass them through to other functions:

```sql
smelt.define monitored_session_rollup(
    source: TableExpr,
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
    metrics: SelectItems<Agg> = (),   -- no context: passed through to session_rollup,
                                      -- which validates against its own context
    alerts: SelectItems<Agg, base> = ()
) -> TableExpr AS (
    WITH base AS (
        smelt.fn.session_rollup(source, user_col, ts_col) {
            metrics: metrics   -- pass through caller's metrics
        }
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

dbt's audience is data analysts and analytics engineers, not programming language enthusiasts. Mandatory `Column<source, Timestamp>` annotations would kill adoption. Unannotated functions that "just work" let people get value immediately. Types become valuable as code matures and gets shared — the same trajectory as TypeScript's gradual typing adoption.

### Implementation Phasing

Tier 1 can ship first — it's just expansion + existing type checking with error tracing. Tier 2 adds checking-mode verification on bodies. Tier 3 adds return type verification. Each tier is independently shippable. The type inference algorithm (§18, bidirectional checking) maps directly onto this phasing: Tier 1 uses pure synthesis mode, Tier 2 adds checking mode at call sites and in bodies, Tier 3 adds checking mode against return types.

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

### The Question

When a function body says `user_col`:

```sql
smelt.define sessionize(
    source: TableExpr,
    user_col: Column<source>,
    ...
) -> TableExpr AS (
    SELECT ... PARTITION BY user_col ...
    FROM source
)
```

Does `user_col` mean the parameter (lexical scoping) or a literal column named `user_col` in the ambient table (dynamic scoping)?

### Decision: Hybrid Scoping (Lexical Parameters + Structural Column Resolution)

The scoping model has two layers, reflecting the fact that SQL fragments inherently reference columns from table contexts:

**Layer 1 — Lexical scoping for parameters.** Function parameters are explicit bindings. Inside a function body, `user_col` refers to whatever column the caller passed — not a literal column named `user_col` in any ambient table. The compiler substitutes the actual column reference during expansion. This is **hygienic expansion** — like Rust macros, not C preprocessor macros.

**Layer 2 — Structural column resolution within table contexts.** Bare column names in SQL expressions resolve against the schemas of `TableExpr` parameters in scope. This is unavoidable — SQL is structurally scoped against its FROM clause. Consider:

```sql
smelt.define add_margin(source: TableExpr) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin
    FROM source
)
```

Here `revenue` and `cost` are not parameters — they resolve from whatever schema `source` carries. This is closer to **row polymorphism** (as in OCaml's object types or PureScript's row types) than pure lexical scoping: the function body is polymorphic over any table that has columns named `revenue` and `cost` of compatible types.

**The honest description:** "Parameters are lexically scoped; column resolution within table-typed parameters is structural (schema-checked but not name-bound)."

When table context bindings are used (e.g., `filters: Predicate<source>`), the compiler checks that column references in the caller's fragment exist in the bound table's schema — making the structural resolution explicit and checked early. Without context bindings, column resolution happens at expansion time.

**Rationale for the hybrid:**
- Functions are **self-contained** — readable without knowing the call site (parameters are lexical, column requirements are visible from the `TableExpr` usage)
- The compiler can **check the body in isolation** (when annotated with context bindings)
- **No surprises** from ambient column names shadowing parameters
- **SQL-native** for the parts that are inherently SQL (column references against tables)

**Trade-off:** The two-layer model is more complex to explain than "everything is lexical." But it matches how SQL actually works — and pretending SQL is lexically scoped would create a different kind of confusion.

> **Note (added §17):** §17 extends row polymorphism to struct-typed *values* (e.g., a column of type `STRUCT(ts TIMESTAMP, user_id TEXT)`). The PLT concept is the same — row variables standing for "plus any other fields" — but the surface types, field-access syntax (`.field` vs. bare column names), and compilation models differ. See §17 for the full treatment.

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
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
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

### Example 2: Session Rollup with Blocks

A reusable model pattern: sessionize events and compute per-session metrics. The caller provides the source table, key columns, and custom metrics.

**Definition:**
```sql
-- functions/patterns/session_rollup.sql
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

smelt.define session_rollup(
    source: TableExpr,
    user_col: Column<source>,
    ts_col: Column<source, Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Predicate<source> = TRUE
) -> TableExpr @deterministic AS (
    WITH sessionized AS (
        smelt.fn.sessionize(source, user_col, ts_col, gap)
    )
    SELECT
        user_col,
        session_id,
        MIN(ts_col) AS session_start,
        MAX(ts_col) AS session_end,
        COUNT(*) AS event_count,
        metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_col, session_id
)
```

**Usage (with blocks):**
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
    user_col => user_id,
    ts_col => event_timestamp,
    gap => INTERVAL '20 minutes'
) {
    metrics:
        SUM(revenue) AS total_revenue,
        COUNT(DISTINCT page_url) AS unique_pages,
        smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event

    filters:
        event_type != 'bot'
        AND user_id IS NOT NULL
}
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
    customer_id_col: Column<source, Integer>,
    product_id_col: Column<source, Integer>
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
        ON source.customer_id_col = c.customer_id
    LEFT JOIN smelt.ref('dim_products') p
        ON source.product_id_col = p.product_id
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
    customer_id_col => customer_id,
    product_id_col => product_id
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
    customer_id_col => customer_id,
    product_id_col => product_id
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

### Functions are additive (decided April 15, 2026)

Introducing functions does not change the meaning of existing models. Models that don't call any function compile and run identically to today. The `smelt.define` and `smelt.fn.*` syntax is purely additive to the grammar.

### Type inference: bidirectional checking with local row unification (decided April 16, 2026)

The type inference algorithm is **bidirectional checking** (Pierce, 2004; Dunfield & Krishnaswami, 2021) with a local unification step for row variables. This is a global design decision: all tiers of the gradual typing system, all row-polymorphic checking (both `TableExpr` and `Struct`), and all error message generation follow from this choice. See §18 for the full rationale, per-tier behavior, and error message properties.

## 13. Open Questions

The decisions in §12 close most of the prior open questions. The items below are the remaining design choices that must be locked in before an implementation plan can be written.

### Block syntax surface (must decide before plan)

The `{ metrics: ... filters: ... }` block introduces a second grammar at the call site, with whitespace-sensitive section delimiters. This is the part of the design most likely to bite the parser, the LSP, and the formatter. Candidates under consideration:

- **Named SQL-like clauses.** Reuse SQL keywords/shape:
  ```sql
  SELECT * FROM smelt.fn.session_rollup(source => ..., user_col => ..., ts_col => ...)
    WITH metrics AS (SUM(revenue) AS total_revenue, ...)
    WITH filters AS (event_type != 'bot')
  ```
  Reads like extra CTEs-as-arguments. Avoids significant whitespace and a nested mini-grammar.
- **Trailing parenthesized labelled blocks.** `) metrics: ( ... ) filters: ( ... )` — still a mini-grammar but inside parens, no significant whitespace.
- **Pure named arguments.** Multi-line parenthesized lists, lean on the formatter. Worst ergonomics, simplest parser.
- **Single trailing positional block (Kotlin-style).** Fine for one fragment param; fails when there are two (metrics + filters).

The choice has knock-on effects on parser complexity, LSP completion behaviour, and formatter rules.

### Grammar for `smelt.define` (must decide before plan)

- One definition per file, or many?
- Frontmatter on function files? (For test markers, future visibility, package metadata.)
- Is `smelt.define ...` a top-level statement that can appear alongside `SELECT`, or are function files restricted to definitions only?

### Annotation syntax (must decide before plan)

`@deterministic`, `@append_only`, `@joins(dim_customers LEFT 1:1)`, `@provenance(...)` are used throughout the paper but never given a formal grammar. Need to decide:

- Where annotations may attach (parameter, return type, whole function)
- The grammar for structured annotation arguments
- Which annotations ship in v1 vs. are reserved

### `Column<T>` parameters: bare refs only, or expressions? (must decide before plan)

Examples show `user_col => user_id`. Decide whether `Column<T>` accepts only bare column references or also computed expressions (`user_col => lower(user_id)`). This has semantic weight when the parameter is spliced into `PARTITION BY` or `GROUP BY`. Recommendation: `Column<T>` is bare-ref-only; use `Expr<T>` for computed values.

### MVP scope (must decide before plan)

- Which fragment sorts ship in v1 (just `Expr<T>`, or also `TableExpr` / `Predicate` / `SelectItems`)?
- Which annotation tier ships first? (Paper currently says Tier 1, but a phased path through Tier 2/3 must be in the plan.)
- Does v1 include any Level 1 planner rule (e.g., the join-elimination showcase from Example 3), or pure expansion only?
- Does v1 include the block syntax, or are expression-only functions enough to validate the architecture?

### Relationship to `smelt.metric()` (must decide before plan)

The language already has `smelt.metric()` with `=>` named parameters. Does `smelt.fn.*` parallel it, or is `smelt.metric` reframed as a special case of `smelt.define`? Affects parser unification.

### Specification tightening (can resolve inside the plan)

- **Union context disambiguation.** For `Column<a | b, Integer>` when both `a` and `b` have an `id` column, what does the caller write? Parameter-name qualification (`a.id`) is the natural answer but needs to be specified explicitly, including parameter-CTE union cases.
- **CTE context checking boundary.** `SelectItems<Agg, sessionized>` can be checked structurally (is it a select list of aggregates?) at definition time, but column-name validation against the CTE schema is call-site-dependent because CTE schemas often include `source.*`. Document the split clearly.

### Already deferred / not blocking

- **LSP block-context completion** — architecturally hard, can land after basic diagnostics.
- **Multiple expansion modes per author** — committed to "the planner's job" unless pain emerges.
- **Function tests** — deferred per §12; functions remain testable through models that use them.
- **Package ecosystem / registry** — not v1.
- **Python model interaction** — functions are SQL-only; Python models are opaque table producers reachable via `smelt.ref()`.

## 14. Summary

smelt functions are **typed, composable SQL fragments** that replace Jinja macros while preserving static analysis:

- **SQL fragment types** ensure structural well-formedness
- **Lexical scoping** makes functions self-contained and analyzable
- **Optional annotations** allow gradual adoption of type rigor
- **Block syntax** provides ergonomic multi-line fragment passing
- **Three-level planner integration** makes functions visible to optimization
- **No recursion** guarantees termination

The design occupies a specific point in the design space: more powerful than simple expression macros, less complex than a full functional programming language. It covers the three categories of Jinja usage that smelt currently lacks (expression reuse, fragment generation, model templates) while maintaining the guarantees that make smelt valuable (type checking, planner visibility, LSP support, testability).

The phased implementation path — Tier 1 annotations first, then Tier 2 and 3 — allows shipping incremental value while building toward the full design.

## 15. PL/Compiler Expert Review

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

## 16. Typing Built-in SQL Functions

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

**5. Modifier clauses on aggregates.** `SUM(x) FILTER (WHERE cond)`, `string_agg(x, ',' ORDER BY y)`, and window `OVER (...)`. These attach `Predicate` / `OrderSpec` fragments to an aggregate call — which the type system already knows how to describe — but the attachment is a syntactic suffix, not a parameter. A refined `AggExpr<T>` that carries optional `filter: Predicate` and `order: OrderSpec` slots would make this explicit.

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

## 17. Struct Parameters and Row Polymorphism for Values

### Motivation

§7 settled on **row polymorphism for `TableExpr` parameters**: bare column names inside a function body resolve structurally against whatever table is passed in, and the compiler checks the required columns exist. That handles the table/row case — columns of a relation.

It does not handle the *value-level* case: a parameter whose type is a **struct-typed column** where the function needs specific fields but not the whole struct. In DuckDB (and Spark, BigQuery, etc.), struct columns are first-class: a table might have `event_data STRUCT(ts TIMESTAMP, user_id TEXT, page TEXT, referrer TEXT)` as a single column. Functions that operate on struct columns face the same brittleness problem as functions that operate on tables — if the struct parameter type is closed, adding a field to the struct breaks every function that accepts it.

This case shows up constantly in analytics SQL once struct-typed columns are in play:

- An aggregate helper that needs a `timestamp` field out of an event struct column but doesn't care what else the struct carries.
- A scoring function that reads `{amount, currency}` out of a transaction struct and ignores everything else.
- A normalizer that extracts and reshapes specific fields from a nested struct while passing other fields through.

If struct parameters are **closed** (either nominally or structurally), every such helper is pinned to one concrete struct type. Add a field to the struct schema and every helper stops accepting it. That's the same brittleness §7 rejected for table parameters — and the fix is the same: **row variables**.

### How this differs from `TableExpr` row polymorphism

`TableExpr` parameters represent **tables** — things that appear in FROM clauses, with columns resolved by name in SQL scope. Row polymorphism for `TableExpr` (§7, plus the Tier 2/2+ annotations) handles "this function works on any table with at least columns X, Y."

`Expr<Struct<{...}>>` parameters represent **struct-typed values** — a single column or expression whose SQL type is a struct. Field access uses dot syntax on the expression (`event_data.ts`), not bare column names in FROM scope. The distinction matters at compilation:

| | `TableExpr` | `Expr<Struct<{...}>>` |
|---|---|---|
| What it represents | A table/relation | A single struct-typed expression |
| Where it appears | FROM clause | Any expression position |
| Field access | Bare column names (`ts`, `user_id`) | Dot syntax (`event.ts`, `event.user_id`) |
| Expansion | Table reference substitution | Expression substitution with field access |

The row-variable mechanism (`..r`, `..`) is the same PLT concept in both cases — but the surface types, field-access syntax, and compilation models are distinct.

### Proposed surface

Extend the existing `Struct<{...}>` type with a **row variable** (`..r`) that stands in for "plus any other fields":

```text
Struct<{ ts: Timestamp, user_id: Text, ..r }>
```

Reads as: a struct with at least `ts: Timestamp` and `user_id: Text`, and any other fields the caller's struct happens to carry. `r` is bound at the function signature — it is not something the caller writes. It exists so that:

1. **Input parameters** can say "I require these fields; pass me any struct that has them."
2. **Return types** can say "I produce at least these fields; the rest pass through from the input."

This is exactly the OCaml object type / PureScript row story, ported to smelt's `Struct<T>`.

### Example A — Reading fields from a struct column

A helper that extracts the hour-of-day from whatever struct has a `ts` field:

```sql
smelt.define event_hour(
    event: Expr<Struct<{ts: Timestamp, ..}>>
) -> Expr<Integer> AS (
    EXTRACT(HOUR FROM event.ts)
)
```

Call sites:

```sql
-- page_events has column: event_data STRUCT(ts TIMESTAMP, user_id TEXT, page TEXT, referrer TEXT)
SELECT smelt.fn.event_hour(event => event_data) AS hour
FROM page_events

-- sensor_readings has column: reading STRUCT(ts TIMESTAMP, device_id TEXT, value DOUBLE)
SELECT smelt.fn.event_hour(event => reading) AS hour
FROM sensor_readings
```

Both are accepted. The checker requires `ts: Timestamp` in the struct; it does not require any particular superset. No overloads, no wrapping, no constructing an intermediate struct.

### Example B — Reading multiple fields from a struct column

A function that checks whether a session gap has occurred, operating on struct-typed event columns:

```sql
smelt.define is_new_session(
    event: Expr<Struct<{user_id: Text, ts: Timestamp, ..}>>,
    gap: Expr<Interval>
) -> Expr<Boolean> AS (
    event.ts - LAG(event.ts) OVER (
        PARTITION BY event.user_id ORDER BY event.ts
    ) > gap
)
```

The signature encodes the exact contract: "I need these two fields on whatever struct you hand me." Adding more fields to the caller's struct type is forward-compatible.

### Example C — Returning a struct with pass-through fields

The harder case is **returning** a struct that preserves the caller's extra fields. This comes up when a function enriches a struct value:

```sql
smelt.define with_hour(
    event: Expr<Struct<{ts: Timestamp, ..r}>>
) -> Expr<Struct<{hour: Integer, ..r}>> AS (
    {hour: EXTRACT(HOUR FROM event.ts), ..event}
)
```

Call site:

```sql
-- event_data is STRUCT(ts TIMESTAMP, user_id TEXT, page TEXT)
SELECT smelt.fn.with_hour(event => event_data) AS enriched
FROM page_events
-- enriched has type: STRUCT(hour INTEGER, ts TIMESTAMP, user_id TEXT, page TEXT)
```

The `..event` spread in the body is the value-level counterpart of the type-level `..r`: it says "carry the remaining fields through unchanged." The same row variable `r` appears in both positions, so the checker knows the output struct's extra fields *are* the input struct's extra fields.

**Projecting specific fields after the call:**

```sql
SELECT
    smelt.fn.with_hour(event => event_data).hour     AS hour,
    smelt.fn.with_hour(event => event_data).user_id  AS user_id
FROM page_events
```

Field access on the returned struct works the same as on any other struct value; the checker has the full output row (explicit fields + row variable bound to the caller's extras) to validate against.

### Compilation model

Row-polymorphic struct parameters **erase at expansion**. The compiler knows the concrete struct type at the call site and generates explicit field references:

```sql
-- Example A expands to:
SELECT EXTRACT(HOUR FROM event_data.ts) AS hour
FROM page_events;

-- Example C expands to:
SELECT {'hour': EXTRACT(HOUR FROM event_data.ts),
        'ts': event_data.ts,
        'user_id': event_data.user_id,
        'page': event_data.page} AS enriched
FROM page_events;
```

Consequences:

- Row variables are resolved at the **call site**, where the caller's concrete struct type is known. At function-definition time, the body is checked against the *declared* fields only; the row variable is opaque inside the body (you cannot enumerate or reflect on `..r`).
- `..event` spread in the return struct expands to the list of the caller's remaining fields, projected in a deterministic order (proposal: declaration order of the caller's struct, with declared return fields first).
- The compiler must support the target engine's struct literal syntax. DuckDB uses `{'field': value, ...}`. If an engine lacks struct literals, `with_hour`-style functions that construct new structs cannot target that engine — the compiler reports this as a backend capability error, not a type error.

### Interaction with existing design

- **Fragment sorts (§3):** unchanged. `Expr<Struct<{...}>>` is still an `Expr`. Row variables are a refinement of `Struct<T>`, not a new sort.
- **Scoping (§7):** complementary, not overlapping. Tables use row polymorphism for bare column names in FROM-scope; struct-typed expressions use row polymorphism for `.field` access. Same PLT concept, different scopes and compilation models.
- **Planner metadata (§8):** `@provenance` annotations extend naturally — a function that spreads `..event` can declare "output fields `..r` come from input `event` field-for-field."
- **Error contract (§6):** Tier 1 errors at call sites look like "field `ts` not found on struct type passed to `event_hour` (struct has: device_id, value, quality_flag)." Tier 2 catches the same error at function-definition boundaries.

### Decisions (April 16, 2026)

1. **Syntax: `..r` for named row variables, `..` for anonymous.** Keeps symmetry with the value-level spread (`{hour: ..., ..event}`), so the type-level `..r` visually matches the value-level `..event` that binds it.

2. **Named row variables are per-function; anonymous `..` is per-parameter.** A named variable like `..r` is bound once at the function signature — if two parameters both declare `..r`, they are constrained to have the same extra fields (useful when two struct arguments must share a shape). Anonymous `..` creates a fresh variable per parameter and can never be referenced elsewhere. This matches OCaml's scoping for object type variables.

3. **One named row variable per function in v1.** Multi-row cases like `merge(a: Expr<Struct<{..r}>>, b: Expr<Struct<{..s}>>) -> Expr<Struct<{..r, ..s}>>` are deferred. When added later, the semantics will be **disjoint union** (PureScript-style): the checker errors if `r` and `s` have overlapping field names. The single-named-variable rule covers essentially all current analytics use cases; the syntax is forward-compatible.

4. **Row-variable mechanism is shared with `TableExpr`, but the parameter types remain distinct.** `TableExpr<{ts: Timestamp, ..r}>` and `Expr<Struct<{ts: Timestamp, ..r}>>` use the same row-variable syntax and unification algorithm, but represent different things (tables vs. struct-typed expressions) with different compilation models (table reference substitution vs. struct field access). This avoids a false unification that would confuse users who work with both tables and struct columns.

5. **No defaults on row-polymorphic parameters in v1.** A parameter whose type contains `..r` (or `..`) must be passed explicitly — no default value allowed. This restriction can be relaxed later (the natural rule would be "default binds `..r` to the empty row").

### Implementation sequencing

**v1: Tier 1 only.** Struct parameters with row variables are checked at expansion time — the compiler infers required fields from `.field` references in the body and checks they exist on the caller's concrete struct type. No row-variable unification algorithm needed; the compiler simply substitutes the concrete type and checks field existence. This is the same strategy §7 uses for bare `TableExpr`.

**Fast-follow: Tier 2/2+ with explicit row variables.** Adds pre-expansion checking and row-variable threading through return types. Requires implementing local row-variable unification (see §18 for the algorithm choice: bidirectional checking with local unification at row-variable binding sites). The syntax is forward-compatible, so Tier 1 functions gain Tier 2 checking by adding annotations without other changes.

The open empirical questions are **expressiveness** (do analytics teams actually have struct-typed columns that benefit from this?), **DX** (does the single-named-variable restriction bite?), and **error message quality** (can row-unification failures be explained clearly?). Tier 1 deployment answers the first question before investing in the unification algorithm.

## 18. Type Inference Algorithm: Bidirectional Checking with Local Row Unification

### The decision (April 16, 2026)

smelt functions will use **bidirectional type checking** as the inference framework, with a **local unification step** at row-variable binding sites. This is a global design decision that shapes how every tier of the gradual typing system (§6) works, how row polymorphism (§7, §17) is checked, and what error messages users see.

### Why bidirectional checking

Three algorithm families were considered:

**Bidirectional checking** (chosen): types flow in two directions — "checking" mode pushes an expected type *down* into an expression, "synthesis" mode computes a type *up* from an expression. At function call sites, the declared parameter type is pushed down; the argument expression is checked against it. At function bodies, parameter types are pushed in, the body synthesizes a result type, and the return annotation (if present) provides a checking target.

**Hindley-Milner / Algorithm W**: collect constraints from the entire function body, solve globally via unification, generalize at let-bindings. Overkill for smelt — there are no higher-order functions, no lambdas, no let-bindings where generalization matters. The only polymorphism is row polymorphism on parameters. Worse, HM's global constraint solving produces *non-local* error messages: when unification fails, the error references two constraints that are individually fine but jointly contradictory, and the source of the conflict may be far from where the error surfaces. This is the famously poor ML/Haskell error experience.

**Constraint-based with ranked heuristics** (TypeScript/Scala 3 style): like HM, but collects constraints and solves with custom ordering to produce better errors by ranking which constraint to blame. Interesting but premature — the error quality advantage comes from ranking complex type relationships (generics with bounds, conditional types, variance). smelt's type relationships are simple enough that bidirectional checking produces good errors without a ranking heuristic.

### How it maps to the three tiers

**Tier 1 (unannotated):** Expand the function, then run the bidirectional checker on the expanded SQL in pure synthesis mode — types flow up from leaves. Errors are mapped back to call-site parameter bindings via the expansion trace. No row variables involved (Tier 1 has no annotations).

**Tier 2 (parameters annotated):** At the call site, push each parameter type into the corresponding argument in checking mode. If the parameter has a row variable (`Struct<{ts: Timestamp, ..r}>`), perform local unification against the concrete argument type — this binds `r` immediately. If a declared field is missing or has the wrong type, report: "expected field `ts: Timestamp` on struct passed to parameter `event`, but struct has: {device_id: Text, value: Double, quality_flag: Boolean}." No deferred constraints. The function body is also checked in isolation: parameter types are pushed in, the body is synthesized bottom-up.

**Tier 2+ (row variable in return type):** After binding `r` at the parameter, substitute into the return type. `Struct<{hour: Integer, ..r}>` becomes `Struct<{hour: Integer, user_id: Text, page: Text}>`. Downstream type checking uses this concrete type. Error messages never mention row variables — the user sees the fully resolved struct type.

**Tier 3 (return type annotated):** Check the function body against the return type in checking mode. Row variables in the return type are still abstract at this point (not bound to a concrete call), so the checker verifies that the body produces *at least* the declared fields. The row variable passes through unchecked — it will be checked at call sites.

### Row unification is local, not global

The critical design property: row-variable unification happens **at the point of use**, not via global constraint solving. When a concrete struct meets a row-polymorphic parameter:

1. Match the declared fields against the concrete fields (check names and types).
2. Bind the row variable to the *remainder* — the concrete fields not matched by declared fields.
3. Substitute the binding forward into any other uses of that row variable (return type, other parameters sharing the same variable).

This is strictly simpler than full HM unification:
- No union-find data structure needed (row variables are solved immediately, not unified incrementally).
- No occurs-check (row variables can't appear in their own binding — structs are not recursive).
- No let-generalization (functions are not first-class values).
- No global constraint propagation (each call site is checked independently).

The main implementation work is step 1 (field matching with subtype checking for compatible types like Text/Varchar) and step 3 (forward substitution into the return type).

### Error message properties

The algorithm guarantees several error-message properties that are worth committing to:

1. **Errors are always local.** Every error references a specific source location where an expected type meets an actual type. There are no "constraint X from line 5 conflicts with constraint Y from line 20" messages.

2. **Row variables never appear in user-facing errors.** By the time an error can occur:
   - At the call site, the row variable is either successfully bound (and the error shows the concrete type) or the binding failed (and the error shows "expected field X, struct has: {concrete fields}").
   - In the function body, the row variable is opaque — errors reference only the declared fields.

3. **Errors say "expected X, got Y."** Every type mismatch can be phrased as two things: what was expected (from the annotation or context) and what was found (from the expression). This is the Rust/TypeScript error experience.

4. **Tier escalation improves error locality without changing error format.** Moving from Tier 1 to Tier 2 moves errors from post-expansion (with traces) to pre-expansion (direct call-site errors). The error *format* is the same ("expected X, got Y"); the *location* gets closer to the source of the problem.

### What this rules out (and why that's fine)

- **Type inference across function boundaries.** A Tier 1 function's return type is not inferred from its body and propagated to callers — it's computed at each call site by expansion. This is intentional: cross-boundary inference creates non-local errors. Functions that want callers to see a stable type declare it (Tier 3).

- **Higher-rank polymorphism.** A parameter cannot itself be polymorphic (e.g., "a function that works for any T"). smelt functions are not higher-order, so this never comes up.

- **Implicit subtyping coercions.** The checker does not silently insert casts. If a parameter expects `Expr<Double>` and the caller passes `Expr<Integer>`, this is a type error, not an implicit coercion. The user writes `CAST(x AS DOUBLE)` explicitly. This keeps the checker simple and errors predictable. (The exception is type *compatibility* checking — `Text` and `Varchar` are treated as the same type, since this is an engine alias, not a coercion.)

### References

- Pierce, B.C. (2004), "Local Type Inference" — the foundational paper for bidirectional checking.
- Dunfield & Krishnaswami (2021), "Bidirectional Typing" — comprehensive survey of the approach.
- Rémy, D. (1994), "Type Inference for Records in a Natural Extension of ML" — row polymorphism with local unification, the direct precedent for smelt's row-variable handling.
- Heeren et al. (2003), "Top Quality Type Error Messages" — analysis of why constraint-based systems produce poor errors and how to improve them (relevant as a "what to avoid" reference).
