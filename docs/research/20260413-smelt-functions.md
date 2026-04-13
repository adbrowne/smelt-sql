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

### Column References with Table Context

The `Column<source, T>` type ties a column reference to a specific table parameter:

```sql
smelt.define sessionize(
    source: TableExpr,
    user_col: Column<source>,         -- any column from source
    ts_col: Column<source, Timestamp> -- timestamp column from source
) -> TableExpr AS (...)
```

This enables the compiler to check that `user_col` and `ts_col` actually exist in whatever `source` the caller provides.

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
    extra_metrics: SelectItems<Agg> = ()
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
    metrics: SelectItems<Agg> = (),
    alerts: SelectItems<Agg> = ()
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

Tier 1 can ship first — it's just expansion + existing type checking with error tracing. Tier 2 adds constraint checking on bodies. Tier 3 adds return type verification. Each tier is independently shippable.

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

### Decision: Lexical Scoping

Parameters are bindings, not literal SQL identifiers. Inside a function body, `user_col` refers to whatever column the caller passed. The compiler substitutes the actual column reference during expansion.

**Rationale:**
- Functions are **self-contained** — readable without knowing the call site
- The compiler can **check the body in isolation** (when annotated)
- **No surprises** from ambient column names shadowing parameters
- **Hygienic expansion** — like Rust macros, not C preprocessor macros

**Trade-off:** Slightly less "SQL-native" feeling. SQL developers expect column names to resolve against the FROM clause. But this is a small price for analyzability, and the mental model is straightforward: "parameters are substituted."

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

Rules rewrite the logical DAG. Functions are **opaque nodes with typed interfaces.** Rules match on function names, parameter types, and declared properties — not on SQL internals.

**What happens here:**
- **Predicate pushdown into blocks.** A downstream WHERE clause is pushed into a function's `filters` parameter.
- **Fusion.** Two adjacent function calls are merged into a single, more efficient function.
- **Pruning.** Unused columns from a function's output are projected away, enabling join elimination (see Example 3 below).
- **Semantic validation.** A function marked `@requires_append_only` is checked against its source.

Functions at this level are like abstract data types: you reason about the interface, not the implementation.

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
    metrics: SelectItems<Agg> = ()
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
    metrics: SelectItems<Agg> = (),
    filters: Predicate = TRUE
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

- **Dynamic schema construction.** You cannot write a function that takes column names as runtime strings and produces a SELECT with those columns. The set of columns must be known at compile time (variadic parameters handle the common cases).
- **Conditional structure.** A function cannot return a JOIN sometimes and a subquery other times based on a runtime value. SQL structure is fixed at compile time. (Conditional *expressions* like CASE/WHEN are fine.)
- **Runtime parameterization of sorts.** Fragment type parameters are compile-time. You can pass runtime values as `Expr` (e.g., `WHERE col > smelt.param('cutoff')`), but you can't choose between different SQL structures at runtime.
- **Recursive patterns.** No function can call itself. Recursive CTEs remain a SQL-level feature, not a function-level one.

These limitations are deliberate. The Jinja use cases that hit them are exactly the ones that produce unmaintainable code.

## 12. Open Questions

### Where do function definitions live?

Options:
- A `functions/` directory in the project (parallel to `models/`)
- Inline in model files (less reusable but convenient)
- A separate `.smelt` file format
- In `smelt.yml` configuration

Leaning toward `functions/` directory with `.sql` extension, mirroring the `models/` convention.

### How does the LSP handle block context?

When a user is writing inside a `metrics:` block, the LSP needs to know which columns are in scope (from the function's expansion context). This requires the LSP to partially expand the function to determine the available columns, then provide completions against that context.

### Should functions support multiple expansion modes?

Should a function author provide both the "full rebuild" and "incremental" versions of their function? Or should that always be the planner rule's responsibility? The latter is cleaner (separation of concerns), but the former might be pragmatic for common patterns.

### Package ecosystem

If smelt functions replace dbt packages (dbt-utils, etc.), there needs to be a mechanism for sharing function libraries across projects. This likely means a package manager and a registry, which is a significant ecosystem investment.

### Interaction with Python models

smelt supports Python models via the `@model` decorator. How do smelt functions interact with Python models? Can a Python model call a smelt function? Can a smelt function reference a Python model's output? The answer is probably: functions are a SQL-layer concept, and Python models are opaque table producers. A function can reference a Python model via `smelt.ref()`, but function definitions are SQL-only.

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

### PLT Techniques Being Used (Named)

1. **Fragment sorts / syntactic categories** (Section 3) — This is the Rust `macro_rules!` technique of *fragment specifiers* (`$e:expr`, `$s:stmt`), which itself derives from the PL concept of **syntactic sorts** in multi-sorted algebras. The idea: a macro system that knows the grammar can restrict substitution to structurally valid positions. MetaML, Template Haskell, and Rust all do this. It's the single strongest idea in the paper.

2. **Staged metaprogramming** (Section 8) — Functions that "compile away" are a one-stage version of **multi-stage programming** (Taha & Sheard, MetaML). The paper correctly identifies Terra as the closest analog. The three-level planner integration is essentially a **multi-pass lowering pipeline** — the same architecture as MLIR (Multi-Level IR), where each dialect level carries different semantic information and rewrites happen at the appropriate level.

3. **Hygienic macro expansion** (Section 7) — Lexical scoping of parameters is **hygienic expansion** (Kohlbecker et al., 1986). The paper draws the right line: Scheme's `syntax-rules`, Rust's `macro_rules!`, versus C's `#define`. Good choice. But see caveats below.

4. **Gradual typing** (Section 6) — The three-tier annotation model is textbook **gradual typing** (Siek & Taha, 2006), following the TypeScript adoption curve. Tier 1 = untyped/duck-typed, Tier 2 = partially typed, Tier 3 = fully typed.

5. **Totality via structural restriction** (no recursion) — This is the Dhall approach, which itself comes from **total functional programming** (Turner, 2004). Banning recursion guarantees termination trivially. The paper correctly notes this is sufficient.

6. **Parametric fragment types** — `Expr<T>`, `Column<source, T>` are a limited form of **dependent types** (the type of `user_col` depends on the *value* of `source`). The paper says "you don't need dependent types" (Section 10, Dhall connection) while actually using a restricted form of them. More on this below.

### Critical Analysis

#### What works well

**The fragment sort system is the right core idea.** Jinja's fundamental failure is operating on strings. The moment you introduce syntactic sorts, you can statically prevent nonsense compositions. Rust's macro system proved this works at industrial scale — the key lesson being that you need *enough* sorts to be useful but not so many that the system becomes its own type theory.

**Late expansion is architecturally sound.** Keeping functions as opaque nodes in the logical plan is the right call. This is exactly how MLIR works — you keep high-level ops as long as possible and lower progressively. The join elimination example (Section 9, Example 3) is a convincing demonstration: once you inline, recovering the semantic structure is a lost cause.

**The gradual annotation strategy is pragmatically correct.** TypeScript's adoption proves this works. The phased implementation (Tier 1 ships first) is the right ordering — you get user adoption before you need the hard type-checking work.

#### Where to push back

**1. `Column<source, T>` is dependent typing in disguise, and it's the hardest part of the design.**

The paper casually introduces `Column<source>` where the type of one parameter depends on the *value* of another parameter. This is a restricted form of dependent types. It's the right design, but the paper undersells the implementation complexity. Consider:

```sql
smelt.define foo(
    a: TableExpr,
    b: TableExpr,
    col: Column<???>   -- which table does this come from?
)
```

You need a system for resolving which table context a column belongs to. What about columns that appear in both tables? What about computed columns from subexpressions of a `TableExpr`? The paper's examples are clean, but real-world usage will hit the edges fast. Haskell's type-level programming and Scala's path-dependent types are warnings about how this kind of "simple" dependent relationship proliferates in complexity.

**Recommendation:** Explicitly scope this as "single-table column provenance only" for v1. Acknowledge that multi-table column resolution (joins inside a `TableExpr` argument) is deferred.

**2. Lexical scoping vs. SQL's inherent dynamic scoping creates a semantic gap.**

SQL is *fundamentally* dynamically scoped — `SELECT user_id FROM events` resolves `user_id` against whatever `events` happens to contain. The paper acknowledges this tension ("slightly less SQL-native") but underestimates it. Consider:

```sql
smelt.define add_margin(source: TableExpr) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin
    FROM source
)
```

Where do `revenue` and `cost` come from? They're not parameters — they're *implicitly* resolved from `source`. This means the function body is *not* actually lexically scoped in the way the paper claims. You have a hybrid: explicit parameters are lexically scoped, but column references within SQL expressions against a `TableExpr` parameter are dynamically resolved. This is closer to **row polymorphism** (as in OCaml's object types or PureScript's row types) than pure lexical scoping.

This isn't fatal, but the paper should name it honestly. The scoping story is: "parameters are lexical; column resolution within table-typed parameters is structural (schema-checked but not name-bound)."

**3. The block syntax introduces a second grammar.**

The `{ metrics: ... filters: ... }` block syntax is effectively a **domain-specific sub-language** embedded in the call site. This has worked before (Ruby blocks, Kotlin DSL builders, Groovy closures in Gradle), but each time it creates a parsing and tooling burden disproportionate to the syntactic convenience.

Specific concerns:
- What's the delimiter between sections? Newlines? Commas? The examples use blank lines, which makes the parser whitespace-sensitive in a context where SQL is not.
- How does error recovery work inside blocks? You now have nested parsing contexts (SQL -> function call -> block -> SQL inside block).
- The LSP complexity note in Section 12 is an understatement — you need *contextual* completion that depends on partial expansion, which is one of the hardest LSP problems.

**Compare with:** Kotlin's trailing lambda syntax, which has the same "pass a block to a function" ergonomic goal but avoids introducing named sections. Consider whether named parameters with parenthesized SQL fragments (the "ugly" version) plus good formatter support might be 80% of the value at 20% of the parser complexity.

**4. The planner integration is the most ambitious and least validated part.**

The three-level planner story is compelling *on paper*, but the join elimination example (Example 3) actually reveals a gap: the planner rule needs to understand the *internal structure* of the function body (which joins exist, which columns come from which join) while the function is supposed to be an "opaque node with typed interface" at Level 1. These are contradictory.

Either:
- Functions are opaque at Level 1, in which case you can't do join elimination without lowering first (making it a Level 2 optimization).
- Functions carry structural metadata (join graph, column provenance map) in their type, in which case "opaque" is a misnomer — you've invented **refinement types** for SQL functions.

The paper is implicitly proposing the second, which is the right design, but should name it. What you actually want is something like:

```
session_rollup : TableExpr
    @joins(dim_customers ON customer_id [LEFT, 1:1],
           dim_products ON product_id [LEFT, 1:1])
    @provenance(customer_segment -> dim_customers.segment,
                product_category -> dim_products.category, ...)
```

This is a **refinement type** — the type carries structural invariants beyond just the sort. That's more implementation work than the paper suggests.

**5. The comparison table undersells Malloy.**

Malloy isn't just "deep semantic modeling" — it's the closest direct competitor to this design. Malloy's `dimension` and `measure` declarations are essentially `Expr<T>` and `AggExpr<T>` with fixed scoping. Malloy's `source` extensions are `TableExpr` transformers. The key difference is that Malloy chose a new query language while smelt extends SQL, but the *function composition model* is very similar. A fair comparison would help readers understand the actual trade-off: Malloy gets cleaner semantics by abandoning SQL syntax; smelt gets migration compatibility by extending SQL syntax but inherits SQL's scoping messiness.

**6. Missing: error message design.**

The paper doesn't discuss error reporting, which is where gradual typing systems succeed or fail. When an unannotated function (Tier 1) is called with wrong types, the user sees an error in the *expanded* SQL with a trace back to the call site. This is the C++ template error experience — notoriously terrible. TypeScript succeeded partly because `any` produces *no* errors rather than *confusing* errors.

The paper should commit to: "Tier 1 errors will show the expansion context but also the call site with parameter mapping, not just a raw SQL error." This is the difference between usable and unusable.

### Historical Precedents Worth Studying

- **SML Functors / OCaml modules** — Parameterized modules that produce types based on input types. `Column<source, T>` is a simplified version of this: a type that depends on a module's signature.
- **Scala's path-dependent types** — `source.Column` where the type depends on a specific value. Ended up being powerful but hard to reason about. Cautionary tale for `Column<source>`.
- **Template Haskell** — Multi-stage compilation where generated code is type-checked after splicing. The Tier 1 strategy is exactly this. TH showed it works but that error messages are the main usability challenge.
- **MLIR's progressive lowering** — The three-level planner is structurally identical to MLIR's dialect-to-dialect lowering. MLIR's lesson: you need clear contracts at each level boundary, which maps to the paper's "return type as safety contract."

### Summary Verdict

The core design is sound. Fragment sorts + late expansion + gradual types is the right combination of PL techniques for this problem. The paper is strongest on motivation and weakest on the interaction between `Column<source>` dependent typing, the hybrid scoping model, and the planner's actual information requirements. The block syntax is a significant engineering investment for a syntactic convenience — defer it until the core function system proves itself.

The biggest risk isn't in the design but in the *implementation ordering*: if Tier 1 ships with C++-template-quality error messages, adoption will stall regardless of how sound the type theory is. Error reporting quality should be an explicit first-class design goal, not an afterthought.
