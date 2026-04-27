# Smelt Functions: Typed SQL Composition to Replace Jinja

**Date:** April 2026
**Status:** Research / Design Exploration
**Author:** Andrew Browne, with design input from Claude

This paper explores a design for **smelt functions**: typed, composable SQL fragments that replace Jinja macros while preserving smelt's static analysis guarantees. The core insight is that if the type system tracks *what kind of SQL fragment* a value is, composition can be free and still statically checked. A deeper insight is that **models and functions are the same concept** -- a typed SQL transformation distinguished only by parameter binding style (DAG-default refs vs explicit parameters) and materialization strategy. This unification extends to SQL built-ins and UDFs as "black box" functions with known signatures but no inspectable bodies. We describe the type system, scoping model, unified model/function concept, gradual annotation strategy, planner integration, and an experimentation roadmap where each step teaches something that informs the next.

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
| `AggExpr<T>` | Expression containing aggregation | SELECT (aggregate context), HAVING |
| `WindowExpr<T>` | Expression containing a window function | SELECT, ORDER BY (not WHERE, GROUP BY, HAVING) |
| `TableExpr` | Something with a schema | FROM, JOIN, WITH |
| `SelectItems` | List of (expression, alias) pairs | SELECT clause |
| `OrderSpec` | Expression + direction | ORDER BY |

These sorts ensure structural well-formedness: you cannot splice a `TableExpr` into a WHERE clause, or an `Expr<Boolean>` into a FROM clause. The compiler checks sort-correctness at each composition point.

### Expression Sort Subtyping

The expression-level sorts form a **linear subtyping chain**:

```
Expr<T>  <:  AggExpr<T>  <:  WindowExpr<T>
```

Anywhere a `WindowExpr<T>` is accepted, an `AggExpr<T>` or plain `Expr<T>` is also valid. Anywhere an `AggExpr<T>` is accepted, a plain `Expr<T>` is valid. This matches SQL's actual restriction rules: a window context (a plain SELECT, where windows are legal) accepts aggregates and scalars; an **aggregate context** (a SELECT with `GROUP BY`, or a SELECT with no `GROUP BY` whose projected expressions are all aggregated — the implicit single-group case — plus `HAVING` in either case) accepts scalars but not window functions; a scalar context (WHERE) accepts only scalars.

This subtyping is the *only* subtyping relationship between sorts. `TableExpr`, `SelectItems`, and `OrderSpec` are unrelated to the expression chain.

A bare column reference like `user_id` is a trivial `Expr<T>` -- there is no separate `Column<T>` sort. This keeps the sort system minimal. If a function parameter is spliced into `PARTITION BY` or `GROUP BY`, the author documents the expectation via naming and comments, not via the type system. (A `Column<T>` subtype could be reintroduced later if experience shows the distinction is valuable, but the simpler model is the right starting point.)

A `Predicate` sort was considered and rejected. Use `Expr<Boolean>` instead -- the positional constraint (WHERE, ON, HAVING) is already enforced by SQL syntax, and one fewer concept to learn is worth more than one more sort.

The initial implementation targets a subset: `Expr<T>` and `TableExpr`. The remaining sorts (`AggExpr<T>`, `WindowExpr<T>`, `SelectItems<K, ctx>`, `OrderSpec`) are added once basic function composition is validated. Each sort is independently addable without breaking existing functions. The linear subtyping chain (`Expr <: AggExpr <: WindowExpr`) is implemented when `AggExpr` is introduced.

### SelectItems Kind

`SelectItems<K, ctx>` carries a **kind** `K` that is the ceiling of the expression sorts contained in the list. The three kinds parallel the expression chain:

- `SelectItems<Scalar, ctx>` — every item is an `Expr<T>`.
- `SelectItems<Agg, ctx>` — items are `Expr<T>` or `AggExpr<T>`.
- `SelectItems<Window, ctx>` — items are `Expr<T>`, `AggExpr<T>`, or `WindowExpr<T>`.

These sorts subtype linearly, mirroring the expression chain:

```
SelectItems<Scalar, ctx>  <:  SelectItems<Agg, ctx>  <:  SelectItems<Window, ctx>
```

A list whose ceiling is `Scalar` also satisfies an `Agg` or `Window` expectation; a list containing a window function does not satisfy an `Agg` expectation. Splice positions require a kind based on the surrounding query shape: `SELECT ${items} FROM t` with no `GROUP BY` and no other aggregates accepts `SelectItems<Window, ctx>` (any kind); `SELECT ${items} FROM t GROUP BY ...` accepts `SelectItems<Agg, ctx>` and rejects `<Window, ctx>`; a position restricted to pure scalars accepts only `<Scalar, ctx>`.

The kind is the *ceiling* of what the list contains, not a uniform property of every element. A list mixing `user_id` (scalar) and `COUNT(*) AS n` (aggregate) has kind `Agg`. Real `SELECT` clauses regularly combine scalars with aggregates under a `GROUP BY`; the ceiling captures this without forcing a per-element tag.

Without this kind parameter, a `SelectItems` value carrying a window function could be spliced into a `GROUP BY` query and the sort system would silently lose the restriction the expression chain was designed to preserve. The kind extends that protection across the list boundary.

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

**Functions compile away.** The target database engine never sees `smelt.fn.*` calls. Everything expands to plain SQL before execution. This is compile-time macro expansion with a sorted type discipline -- inspired by staged code generation systems like Terra (Devito et al.) and MetaML (Taha & Sheard), though smelt's model is simpler: a single expansion phase, not the multi-stage programming those systems provide.

**Named parameters follow PostgreSQL.** The `param => value` syntax follows PostgreSQL's named notation for function calls (supported since PostgreSQL 9.5). Oracle PL/SQL uses the same `=>` convention.

**No recursion.** Functions cannot call themselves, directly or indirectly. This guarantees termination -- the same totality property that makes Dhall work as a configuration language. Dhall demonstrated that a non-Turing-complete language with a modest type system can replace complex templating in practice. smelt's type system (SQL types plus fragment sorts) is the equivalent of Dhall's records and unions.

**Composition is free.** Functions can call other functions to any depth. A calls B calls C. The only restriction is no cycles, which guarantees finite expansion:

```sql
smelt.define sessionize(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
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

## 4. The Unified Model -- Models as Functions

A smelt model is a SQL file that takes `TableExpr` inputs (via `smelt.ref()` and `smelt.source()`) and produces a `TableExpr` output. A smelt function is a `smelt.define` block that takes typed fragment inputs and produces a typed fragment output. These are the same concept with different defaults.

### The Equivalence

Consider a typical model:

```sql
-- models/margins.sql
---
materialization: table
---
SELECT revenue - cost AS margin
FROM smelt.ref('product_summary')
```

Under the unified view, this is equivalent to:

```sql
smelt.define margins(
    product_summary: TableExpr = smelt.ref('product_summary')
) -> TableExpr AS (
    SELECT revenue - cost AS margin
    FROM product_summary
)
```

The model's `smelt.ref()` calls are parameters with default values drawn from the project DAG. The materialization decision (`table`, `view`, `ephemeral`, `materialized_view`) is orthogonal -- a property of how the output is persisted, not what the transformation is.

### Evidence: This Is Already How It Works

**Ephemeral models are transparent inline functions.** An ephemeral model's SQL is inlined as a CTE into every downstream model that references it -- the same expansion mechanics as `smelt.define` functions. The only difference is parameter binding: ephemeral models get their inputs from the DAG; functions get theirs from explicit call sites.

**Testing already subverts refs.** The testing framework provides mock tables with only the columns a model actually touches, proving the model's real dependency is "a table with columns X, Y, Z of compatible types" -- not "specifically the output of model `product_summary`." This is row polymorphism discovered empirically.

### Two Orthogonal Properties

Every SQL transformation in smelt has two independent properties:

**Transparency:** Can smelt see the body?
- *Transparent* -- body is available for expansion, type checking, and planner optimization across boundaries. All user-authored SQL (models and `smelt.define` functions) is transparent.
- *Black box* -- only the signature (input types, output type) is known. SQL built-ins (`SUM`, `LOWER`), UDFs, external functions. The planner treats these as atomic nodes.

**Materialization:** How is the output persisted?
- *Table / View / Materialized view* -- output is stored or computed by the engine. Scheduled in the DAG. Referenceable via `smelt.ref()`.
- *Inline (ephemeral)* -- expanded at compile time. No persistent output.

### The Taxonomy

| Current concept | Transparency | Materialization | Parameters |
|---|---|---|---|
| Table/view model | Transparent | Persisted | DAG-default (refs/sources) |
| Ephemeral model | Transparent | Inline (CTE) | DAG-default |
| `smelt.define` function | Transparent | Inline (expansion) | Explicit |
| SQL built-in | Black box | Inline | Engine-provided signature |
| UDF | Black box | Inline | User-declared signature |
| Source | Black box | External | Schema from YAML/catalog |

The model/function distinction reduces to parameter binding style (DAG-default vs explicit) -- sugar, not a fundamental concept. The deeper axes are transparency and materialization.

### Parameterized Models

The unified view makes parameterized models natural. A model that takes explicit `TableExpr` parameters alongside its DAG-default refs allows the same logic to be reused across different source tables (regional variants, A/B test arms, environment-specific sources):

```sql
-- models/regional_revenue.sql
---
materialization: table
---
SELECT
    region, SUM(amount) AS total_revenue
FROM smelt.ref('orders', default => smelt.source('us_orders'))
GROUP BY region
```

A caller (or test) can override the default, providing a different table without changing the model. This is a smooth continuum: at one end, a fully-hardcoded model with all refs; at the other, a fully-parameterized function with no DAG defaults.

### Implications for the Planner

The planner already operates on the model DAG. Under the unified view, this DAG includes all materialized transformations (models) as nodes and all inline transformations (functions, ephemeral models) as expansions within nodes. The planner's optimization boundary aligns with transparency: it can reason across transparent boundaries (rewrite, fuse, push filters) but must treat black box boundaries as atomic.

## 5. Black Box Functions -- Signatures Without Bodies

Black box functions are transformations where smelt knows the type signature but cannot inspect the body. They are the counterpart to transparent functions (where smelt sees and expands the SQL body).

### Why Black Box Matters

Models call SQL built-in functions far more often than user-defined functions. If built-ins carry the same fragment-typed signatures as user functions, every SQL call in every model gets compile-time checking, hover types, and completion. This is not an optional extension -- it is mandatory for useful type checking. A type system that checks `smelt.fn.safe_divide()` but not `SUM()` or `COALESCE()` covers a small fraction of real SQL.

### Categories of Black Box Functions

**SQL built-ins** are provided by the target engine. smelt ships a signature registry per dialect (DuckDB, Spark, PostgreSQL). Most built-ins have simple signatures that fit the existing fragment sort system:

| Built-in shape | Signature |
|----------------|-----------|
| Pure scalar (`LOWER`, `ABS`, `LENGTH`) | `Expr<T1> -> Expr<T2>` |
| Binary scalar (`POWER`, `MOD`) | `(Expr<T>, Expr<T>) -> Expr<T>` |
| Aggregates (`SUM`, `COUNT`, `AVG`) | `Expr<T> -> AggExpr<T>` |
| Predicate-producing (`IS NULL`, `LIKE`) | `Expr<T> -> Expr<Boolean>` |
| Simple table functions (`generate_series`) | `(Expr<Int>, Expr<Int>) -> TableExpr` |

**User-declared external functions** (UDFs, Snowflake external functions, Python UDFs) are declared by the user with explicit signatures:

```sql
smelt.extern my_ml_model(
    features: Expr<Struct<{age: Integer, income: Double, ..}>>
) -> Expr<Double>
```

The function name passes through to the generated SQL unchanged. No expansion, no body analysis -- just signature checking at call sites.

**Sources** are black box `TableExpr` producers with zero parameters and a schema declared in YAML or inferred from a catalog.

### The Signature Language for Black Box Functions

Black box functions are always fully annotated -- there is no body to infer from. This means the signature language must be expressive enough to describe what the ~20% of SQL built-ins that need extensions require:

1. **Type parameters (generics).** `COALESCE<T>(Expr<T>, Expr<T>) -> Expr<T>` returns the common supertype. `ARRAY_AGG<T>(Expr<T>) -> AggExpr<Array<T>>` wraps element type.
2. **Variadics.** `COALESCE`, `CONCAT`, `GREATEST` accept arbitrary arity. The signature language needs `Expr<T>...` or equivalent.
3. **Types as arguments.** `CAST(x AS INTEGER)`, `EXTRACT(YEAR FROM ts)`. Not expressible as `Expr<T>`. Options: a `Type` parameter sort, or primitive grammar handling for these specific forms.
4. **Keyword-argument syntax.** `TRIM(BOTH ' ' FROM x)`, `SUBSTRING(s FROM 1 FOR 3)`. These are SQL grammar constructs, not function calls in the usual sense. Treated as primitive grammar, not as generic black box signatures.
5. **Modifier clauses.** `SUM(x) FILTER (WHERE cond)`, `OVER (...)`. Syntactic suffixes on aggregate calls that modify behavior.
6. **Schema-returning table functions.** `UNNEST(array_col)` depends on element type. `read_csv` with auto-schema is not compile-time typeable.

Categories 1-3 require extensions to the signature language. Categories 4-5 are SQL grammar handled by the parser. Category 6 is untypeable without schema hints.

### What Remains Untypeable

- Auto-schema built-ins without a schema hint (`read_csv('x.csv')`)
- Dynamic `EXECUTE` / string-templated SQL
- Untyped JSON navigation (`col->>'foo'`) -- typeable only as unconditional `Text`

### Gradual Typing Interaction

Black box functions do not participate in the three-tier gradual typing system (section 8). They are always "Tier 3" -- fully annotated -- because the signature is all that exists. The gradual typing tiers apply only to transparent functions, where the author can choose how much to annotate.

This means a Tier 1 transparent function (unannotated) calling a black box built-in gets the best of both worlds: the built-in's return type is known from the registry, providing type information that flows into the Tier 1 expansion check. `SUM(revenue)` has a known return type even if the function containing it has no annotations.

### Planner Implications

The planner treats black box functions as optimization barriers -- it cannot rewrite what's inside. However, black box functions can still carry declared properties (in frontmatter, per §16 decision 22):

- `deterministic: true` -- the planner knows the function produces the same output for the same input
- `idempotent: true` -- safe to retry
- SQL built-ins inherit properties from the registry (e.g., `SUM` is deterministic, `RANDOM()` is not)

The planner can reason *around* black box functions (push a filter below a black box scalar function, eliminate unused columns before a black box table function) but never *through* them.

## 6. Context Bindings -- Controlling What Callers Can See

When a function takes a fragment-typed parameter that contains column references, a natural question arises: which columns can it reference? The answer is **context bindings** -- annotations that declare which table context a fragment's columns resolve against.

Any fragment sort that can contain column references may declare a context:

| Sort | With context binding | Meaning |
|------|---------------------|---------|
| `Expr<T>` | `Expr<T, source>` | Scalar expression whose columns come from `source` |
| `SelectItems` | `SelectItems<Agg, sessionized>` | Aggregate select items over `sessionized` columns |
| `OrderSpec` | `OrderSpec<enriched>` | Ordering expression over `enriched` columns |

The parameterization convention: `Expr<SqlType, Context?>`, `SelectItems<Kind?, Context?>`, `OrderSpec<Context?>`. The compiler disambiguates context names from type names by checking whether the identifier refers to a `TableExpr` parameter or CTE in scope; if it does, it is a context binding, otherwise it is a type.

A context can be:
1. **A `TableExpr` parameter** -- e.g., `Expr<Integer, source>` where `source` is a parameter
2. **A CTE defined in the function body** -- e.g., `SelectItems<Agg, sessionized>` where `sessionized` is a `WITH` clause

### Context Inference from Splice Points

Context bindings are **optional annotations, not required declarations.** The compiler infers a parameter's context from where it is spliced in the function body. If `filters` appears in `WHERE filters` and the `WHERE` applies to a `FROM source` clause, the compiler infers that `filters` has context `source`.

**When a parameter is spliced in multiple places,** the available columns are the **intersection** of the schemas at each splice point -- only columns that appear in all locations with the same name and compatible type. This is the safe default: the parameter's fragment can only reference columns that are guaranteed to exist everywhere it is used.

For example, if `pred` is spliced into both `WHERE pred` (over a table with columns `{id, name, amount}`) and `HAVING pred` (over a grouped result with columns `{id, total}`), the parameter can only reference `id` -- the one column present in both scopes with the same type.

Explicit context annotations serve two purposes:
1. **Documentation** -- making the contract visible in the signature
2. **Validation** -- the compiler checks the annotation matches the inferred context (or intersection)

### The Key Insight: Asymmetric Access Control

Consider `session_rollup`:

```sql
---
deterministic: true
---
smelt.define session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean> = TRUE
) -> TableExpr AS (
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

The compiler infers that `metrics` has context `sessionized` (spliced into `SELECT ... FROM sessionized`) and `filters` has context `source` (spliced into `WHERE` on the outer query, which scans `sessionized` -- but `sessionized` is derived from `source`). The `metrics` annotation is explicit here because the caller needs to know they can reference `session_id` in their aggregate expressions. The `filters` annotation is omitted -- the compiler infers it from the splice point.

**The author controls what each caller-provided fragment can see.** This is capability-based access control applied to SQL column namespaces. Narrowing the context prevents callers from depending on internal implementation details.

### CTE-Derived Contexts

CTE context is computed, not declared. The author references a CTE by name in the type annotation; the compiler derives its schema from the body. This means the signature and body are coupled -- changing the CTE's SELECT list may change what callers can reference. This coupling is intentional: the context binding names the *splice-point scope*, and the splice point is in the body.

Computing the output schema of a CTE uses the same schema inference the type system already performs for models and function calls. The incremental work is wiring CTE schema computation into the context binding checker, not building a new analysis from scratch.

### The No-Overlap Rule (Replaces Union Contexts)

An earlier design allowed union contexts (`Expr<Boolean, source | customers>`) for parameters that reference columns from multiple joined tables. This was rejected because it reintroduces the join ambiguity problem that SQL already struggles with -- two tables with an `id` column require complex disambiguation rules.

Instead, **parameter contexts must have unique column names.** When a function joins multiple tables, the author has three strategies for exposing columns to caller-provided fragments:

**Strategy 1: Explicit SELECT into a CTE.** Create a CTE with the specific columns the caller needs, aliased to avoid collisions:

```sql
smelt.define enrich_order(
    source: TableExpr,
    customer_id_col: Expr<Integer>,
    product_id_col: Expr<Integer>,
    extra_cols: SelectItems<enriched> = ()
) -> TableExpr AS (
    WITH
        customers AS (SELECT * FROM smelt.ref('dim_customers')),
        products AS (SELECT * FROM smelt.ref('dim_products')),
        enriched AS (
            SELECT
                source.*,
                c.segment AS customer_segment,
                c.country AS customer_country,
                p.category AS product_category
            FROM source
            LEFT JOIN customers c ON customer_id_col = c.customer_id
            LEFT JOIN products p ON product_id_col = p.product_id
        )
    SELECT enriched.*, extra_cols
    FROM enriched
)
```

The caller's `extra_cols` fragment sees the `enriched` CTE's flattened, unambiguous schema.

**Strategy 2: Typed TableExpr parameter.** Require the caller to pass a pre-joined or pre-selected table:

```sql
smelt.define summarize(
    source: TableExpr<{region: Text, amount: Numeric, ..}>
) -> TableExpr AS (...)
```

**Strategy 3: `smelt.as_struct()` for compile-time namespacing (deferred to post-v1 — see §16 decision 19).** When multiple tables need to be accessible without column name collisions, a future `smelt.as_struct()` construct would wrap each table's columns into a struct:

```sql
smelt.as_struct(source EXCEPT customer_id, product_id)
-- produces: STRUCT(col1, col2, ...) excluding the join keys
```

This is intended as a **compile-time construct** with zero runtime cost -- the compiler would know the concrete struct fields at expansion time and generate explicit field references, providing SQL-safe namespacing (`source_struct.revenue`, `customer_struct.segment`) without runtime struct creation overhead. In v1, use Strategy 1 or Strategy 2; `smelt.as_struct()` is revisited alongside Step 8 (struct row polymorphism, §11) so the two surfaces can be designed together.

### Remaining Edge Cases

CTE context checking is partially call-site-dependent: `SelectItems<Agg, sessionized>` can be structurally checked (is it a select list of aggregates?) at definition time, but column-name validation requires knowing the call-site schema when the CTE includes `source.*`.

## 7. Parameters-First Scoping

When a function body says `user_col`, does it mean the parameter or a literal column named `user_col`?

### Resolution Order

Bare names in a function body resolve in this order:

1. **Parameters first.** If `user_col` is a parameter name, it refers to the parameter -- always.
2. **SQL FROM scope second.** If no parameter matches, the name resolves against the schemas of `TableExpr` parameters in scope (standard SQL column resolution).

This is a deliberate departure from pure SQL scoping, where all bare names resolve against FROM. Parameters take priority because they are the function's explicit interface -- the author chose the name, and the caller bound a value to it. Letting a table column silently shadow a parameter would be a source of subtle bugs.

### Shadow Warnings

When a parameter name collides with a column from a `TableExpr` in scope, the compiler emits a **warning** (not an error). The parameter still wins, but the author is alerted that a column is being shadowed. This catches the common case where a function names a parameter `user_id` and the source table also has a `user_id` column -- after expansion, the parameter's bound value is spliced in, and the table column is inaccessible by that name.

To access the shadowed column, the author qualifies it: `source.user_id`.

### Bare Columns from TableExpr

Bare column references from `TableExpr` parameters are allowed when **unambiguous** -- consistent with the no-overlap rule (§6). If only one `TableExpr` in scope has a column named `revenue`, the bare reference `revenue` resolves to it:

```sql
smelt.define add_margin(source: TableExpr) -> TableExpr AS (
    SELECT source.*, revenue - cost AS margin
    FROM source
)
```

Here `revenue` and `cost` are not parameters -- they resolve from whatever schema `source` carries. This is **row polymorphism** (Remy, 1994; OCaml object types; PureScript row types): the function body is polymorphic over any table that has columns named `revenue` and `cost` of compatible types.

Qualification is required only when a bare column name overlaps with a parameter name (in which case the parameter wins by default, and `source.column_name` accesses the column).

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

**The honest description:** "Parameters resolve first; bare column names resolve against SQL FROM scope when no parameter matches. Annotations make structural column requirements explicit. Shadow warnings catch collisions."

## 8. Gradual Typing -- Three Tiers of Annotation

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

## 9. Type Inference -- Bidirectional Checking

smelt functions use **bidirectional type checking** (Pierce & Turner, 2000; Dunfield & Krishnaswami, 2021) with a local unification step at row-variable binding sites.

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
- **No implicit coercion for concrete parameter types.** If a user function parameter expects `Expr<Double>` and the caller passes `Expr<Integer>`, this is a type error. The user writes `CAST(x AS DOUBLE)`. (Exception: engine aliases like `Text`/`Varchar` are treated as the same type.) However, **type constraints** (`Numeric`, `Ordered`) accept any type that satisfies them -- this is constraint satisfaction, not coercion. And **built-in operators/functions** that accept multiple numeric arguments compute the least upper bound (LUB) of the argument types via the promotion chain (§16, decision 9) -- e.g., `COALESCE(Integer, Double)` returns `Double`.

## 10. Block Syntax -- Ergonomic Fragment Passing

Passing multi-line SQL fragments as inline function arguments is syntactically awkward. Block syntax uses the `PASSING` keyword with named `name AS (...)` clauses trailing a function call:

```sql
SELECT * FROM smelt.fn.session_rollup(
    source => smelt.ref('web_events'),
    user_col => user_id,
    ts_col => event_timestamp,
    gap => INTERVAL '20 minutes'
)
PASSING metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages,
    smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event
)
PASSING filters AS (
    event_type != 'bot' AND user_id IS NOT NULL
)
```

Each `PASSING name AS (...)` clause binds a fragment-typed parameter by name. The compiler treats it identically to inline arguments. `PASSING` is borrowed from SQL/XML (`XMLTABLE ... PASSING ...`), where it serves the same purpose: binding values into a parameterized context. Using a distinct keyword avoids the CTE collision problem -- `WITH name AS (...)` would be ambiguous with SQL's CTE syntax. Block clauses must trail the function call's closing `)` directly.

### Blocks Compose

A function can receive blocks from its caller and pass them through:

```sql
smelt.define monitored_session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    metrics: SelectItems<Agg> = (),
    alerts: SelectItems<Agg, base> = ()
) -> TableExpr AS (
    WITH base AS (
        smelt.fn.session_rollup(source, user_col, ts_col)
        PASSING metrics AS (metrics)
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

Using `PASSING` instead of `WITH` eliminates the CTE ambiguity entirely -- the parser can distinguish `PASSING` from `WITH` without knowing the function's parameter names, which means the parser operates independently of the type checker. Named parameters with parenthesized fragments (the "ugly" version from the examples) remain available as the fallback -- blocks are pure sugar.

## 11. Row Polymorphism for Struct Values

The parameters-first scoping model (section 7) handles row polymorphism for `TableExpr` parameters -- "this function works on any table with at least columns X, Y." But struct-typed columns (DuckDB, Spark, BigQuery) face the same brittleness problem at the *value* level: if struct parameters are closed, adding a field to the struct breaks every function that accepts it.

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

## 12. Planner Integration -- Three Levels

This is where smelt functions differ most fundamentally from Jinja macros. In dbt, macros are expanded to text before anything sees them. In smelt, functions are **visible to the planner as first-class nodes** with typed interfaces and declared properties.

### Why Not Just Expand?

If functions were expanded to plain SQL before the planner runs, the planner loses semantic information. It sees `SUM(CASE WHEN ts - LAG(ts) OVER (...) > INTERVAL '30 minutes' THEN 1 ELSE 0 END) OVER (...)` instead of knowing "this is a session rollup." Pattern-matching on raw SQL to rediscover this structure is fragile. Keeping functions in the IR means planner rules match on **function names and properties**, not SQL patterns.

### Level 1: Logical -> Logical (pre-expansion)

Rules rewrite the logical DAG. Functions are nodes with rich typed interfaces carrying **enriched type annotations** -- structural metadata beyond the basic fragment sort. This is conceptually related to refinement types (Rondon et al., 2008) in that types carry information beyond the basic sort, though the mechanism here is explicit annotation rather than logical predicates verified by a solver.

The compiler analyzes function bodies and attaches structural metadata:
- **Column provenance map:** Which output columns come from which input tables
- **Join graph:** Which tables are joined, join type, cardinality
- **Declared properties:** `deterministic`, `idempotent`, `append_only` (frontmatter keys per §16 decision 22)

This metadata enables filter pushdown, function fusion, join elimination, and semantic validation -- all by reasoning about the typed interface, never pattern-matching on SQL.

In v1, metadata is **explicitly declared** by function authors (`joins:` / `provenance:` keys in frontmatter, shape TBD when they land) rather than automatically derived. Automatic derivation requires a full lineage analyzer -- a substantial compiler component. Explicit declarations let the planner integration ship without this, while keeping the door open to add automatic derivation later as a DX improvement. In practice, only "model template" functions benefit from planner-level optimization, so the declaration burden is concentrated on a small number of high-value functions.

However: v1 frontmatter properties are simple booleans and lists only (`deterministic`, `idempotent`, `append_only`, `backends`). Structured properties (`joins`, `provenance`) are deferred until the planner actually needs them. Per decision 22, all properties live in the frontmatter block that immediately precedes the declaration.

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

Function properties still matter: `idempotent: true` tells Level 3 that retry is safe; `deterministic: true` tells it re-execution produces the same result.

### What Ships When

**MVP (Steps 1-5):** Pure expansion for transparent functions, signature checking for black box functions. No planner rules at any level. Simple frontmatter properties (`deterministic`, `idempotent`, `append_only`) are parsed and stored but not acted on.

**Post-MVP (Step 7):** Level 1 planner rules. Structured frontmatter properties (`joins`, `provenance`) for the functions that benefit from optimization. The transparency rule guides which functions the planner can reason through. Levels 2 and 3 build on existing planner infrastructure.

This separation lets the function system be validated as a composition mechanism before adding planner complexity.

### The Transparency Rule

The unified model (section 4) introduces a clean optimization boundary: the planner can reason across **transparent** function boundaries (rewrite, fuse, push filters through) but must treat **black box** functions as atomic nodes. This simplifies the planner story:

- Transparent functions: the planner sees the body, can analyze joins, trace column provenance, eliminate unused work.
- Black box functions (SQL built-ins, UDFs): the planner sees only the signature and declared properties. It can reason *around* them (e.g., push a filter below a black box scalar) but never *through* them.
- The materialization boundary (table/view vs ephemeral/inline) determines which transparent functions are expanded before planning vs. which persist as DAG nodes.

## 13. Built-in Function Typing -- Subsumed by Black Box Functions

*Note: This section is retained for detailed analysis. The conceptual framework has moved to section 5 (Black Box Functions), which treats built-in typing as a special case of the broader black box function concept.*

Built-in SQL function typing is **mandatory for useful type checking**, not an optional extension. Models call built-in SQL functions far more often than user functions. A type system that checks `smelt.fn.safe_divide()` but not `SUM()` or `COALESCE()` covers a small fraction of real SQL.

Under the unified model (sections 4-5), SQL built-ins are black box functions with engine-provided signatures. The analysis below identifies which built-ins fit the existing signature language and which require extensions to the black box signature language.

### What Fits the Existing Signature Language (~80%)

| Built-in shape | Signature |
|----------------|-----------|
| Pure scalar (`LOWER`, `ABS`, `LENGTH`) | `Expr<T1> -> Expr<T2>` |
| Binary scalar (`POWER`, `MOD`) | `(Expr<T>, Expr<T>) -> Expr<T>` |
| Aggregates (`SUM`, `COUNT`, `AVG`) | `Expr<T> -> AggExpr<T>` |
| Predicate-producing (`IS NULL`, `LIKE`) | `Expr<T> -> Expr<Boolean>` |
| Simple table functions (`generate_series`) | `(Expr<Int>, Expr<Int>) -> TableExpr` |

### What Requires Signature Language Extensions (~20%)

1. **Generics / type parameters.** `COALESCE(a, b, c)` returns the common supertype. `ARRAY_AGG(x)` returns `Array<T>`. Requires type parameters on black box signatures.
2. **Variadics.** `COALESCE`, `CONCAT`, `GREATEST` accept arbitrary arity. The signature language needs `Expr<T>...` or equivalent.
3. **Types as arguments.** `CAST(x AS INTEGER)`, `EXTRACT(YEAR FROM ts)`. Not expressible as `Expr<T>`. Options: a `Type` parameter sort, or primitive grammar handling for these specific forms.
4. **Keyword-argument syntax.** `TRIM(BOTH ' ' FROM x)`, `SUBSTRING(s FROM 1 FOR 3)`. SQL grammar constructs, not generic function calls. Treated as primitive grammar.
5. **Modifier clauses.** `SUM(x) FILTER (WHERE cond)`, `OVER (...)`. Syntactic suffixes on aggregate calls.
6. **Schema-returning table functions.** `UNNEST(array_col)` depends on element type. `read_csv` with auto-schema is not compile-time typeable.

Categories 1-3 are on the critical path -- the signature language for black box functions must support them. Categories 4-5 are SQL grammar handled by the parser. Category 6 is untypeable without schema hints.

### What Remains Untypeable

- Auto-schema built-ins without a schema hint (`read_csv('x.csv')`)
- Dynamic `EXECUTE` / string-templated SQL
- Untyped JSON navigation (`col->>'foo'`) -- typeable only as unconditional `Text`

### Implementation: One Canonical Registry, Not Per-Dialect (decided April 18, 2026)

Built-in SQL functions use a **single canonical signature registry** with backend compatibility expressed as a function property — not per-dialect registries.

**Three tiers of function portability:**

| Tier | Description | Backend handling | Example |
|------|-------------|-----------------|---------|
| **1: Fully portable** | Identical semantics across backends; at most name/syntax differences | Backend translation layer remaps names and minor syntax (`EVERY` → `BOOL_AND` on DuckDB) | `COUNT`, `SUM`, `UPPER`, `COALESCE`, `ROW_NUMBER` |
| **2: Portable with translation** | Same concept but argument conventions or format strings differ | Backend translation rewrites arguments, translates format strings | `STRING_AGG`/`LISTAGG`, `DATE_ADD`/`DATEADD`, JSON functions |
| **3: Engine-specific** | Only exists on one backend, or semantics diverge too far | Namespaced — `duckdb.read_parquet()`, `spark.from_json()` — model is pinned to a backend | `read_parquet`, `explode_outer`, `list_transform` |

**Canonical types enforced with CAST.** The canonical return type is always enforced via CAST in generated SQL. `SUM(Integer)` has canonical return type `BigInt`; on PostgreSQL (which natively returns `Decimal(38,0)`), the generated SQL is `CAST(SUM(col) AS BIGINT)`. This prevents backend-specific precision from silently leaking into downstream table schemas — critical for ETL where output schemas are a contract.

**Backend namespace for native precision.** If a user wants the higher precision a specific backend offers, they use the backend namespace: `postgres.sum(col)` returns `Decimal(38,0)` with no enforced cast. This is an explicit opt-in to backend-specific behavior that marks the model as non-portable. The same namespace mechanism handles UDFs, which are inherently backend-specific.

**`backends` as a function property.** Backend compatibility is a frontmatter property (per §16 decision 22), the same mechanism that carries `deterministic` and `idempotent`. The canonical built-in registry stores each entry's properties in its own data format, but the conceptual shape is identical to a user-declaration's frontmatter:

```
COALESCE(a, b)                   # deterministic: true, backends: all
MEDIAN(col)                      # deterministic: true, backends: [duckdb, postgres]
duckdb.read_parquet('*.parquet') # backends: [duckdb]   (implied by the duckdb.* namespace)
```

This means the planner can answer "can I move this model to Spark?" with the same query pattern as "is this model deterministic?" — check whether all functions have the required property. Diagnostics like "MEDIAN is not available on Spark" use the same machinery as other type errors.

**Why not per-dialect registries:** A per-dialect registry would require models to be associated with a dialect to resolve function signatures, tying models to backends even when they use only portable functions. The canonical registry keeps models backend-agnostic by default — backend-specificity is an explicit choice (via namespace) that the planner can see and reason about.

## 14. Concrete Examples

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
PASSING metrics AS (
    SUM(revenue) AS total_revenue,
    COUNT(DISTINCT page_url) AS unique_pages,
    smelt.fn.safe_divide(SUM(revenue), COUNT(*)) AS revenue_per_event
)
PASSING filters AS (
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

Properties flow through all three levels: `deterministic: true` tells Level 3 replaying a failed batch is safe; `append_only: true` on `source` tells Level 2 incremental processing is valid.

### Example 3: Join Elimination via Function-Aware Planning

*Note: This example illustrates a future capability (Step 7 in the roadmap). It requires planner integration with structured frontmatter properties (`provenance`, `joins`), which are post-MVP.*

This demonstrates why planner-visible functions enable optimizations that blind expansion cannot.

**Setup:** `enrich_order` (defined in section 6) joins a fact table to customer and product dimensions via LEFT JOINs with unique keys (1:1 cardinality).

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

## 15. Limits of the Design

Even at maximum ambition, some things remain outside scope:

- **Dynamic schema construction.** You cannot write a function that takes column names as runtime strings and produces a SELECT with those columns. The set of columns must be known at compile time. (`SelectItems` parameters cover the "list of things" case without requiring variadics.)
- **Conditional structure.** A function cannot return a JOIN sometimes and a subquery other times based on a runtime value. SQL structure is fixed at compile time. (Conditional *expressions* like CASE/WHEN are fine.)
- **Runtime parameterization of sorts.** Fragment type parameters are compile-time. You can pass runtime values as `Expr` (e.g., `WHERE col > smelt.param('cutoff')`), but you can't choose between different SQL structures at runtime.
- **Recursive patterns.** No function can call itself. Recursive CTEs remain a SQL-level feature, not a function-level one.

These limitations are deliberate. The Jinja use cases that hit them are exactly the ones that produce unmaintainable code.

## 16. Design Decisions (April 18, 2026)

The following decisions resolve open questions from earlier drafts. Each is applied throughout the paper; this section documents the rationale in one place.

### 1. Parameters-first scoping (§7)

Bare names in a function body resolve to parameters first, then SQL FROM scope. Shadow warning on collisions.

**Rationale:** The earlier "hybrid scoping" model said parameters were lexical and columns were structural, but left the priority ambiguous during Tier 2 body checking. Parameters-first is the unambiguous rule: the author chose the parameter name, so it wins. The shadow warning catches the common footgun where a parameter name accidentally matches a column. Qualification (`source.column_name`) provides the escape hatch.

### 2. Column\<T\> dropped — use Expr\<T\> everywhere (§2)

A bare column reference like `user_id` is a trivial `Expr<T>`. There is no separate `Column<T>` sort.

**Rationale:** The distinction between "bare column reference" and "scalar expression" added complexity without proportional value. `Column<T>` existed to prevent computed expressions in `PARTITION BY` / `GROUP BY` positions, but SQL handles this fine — the engine reports an error if needed. One fewer sort means one fewer concept for function authors to learn. `Column<T>` can be reintroduced as a subtype of `Expr<T>` later if experience shows the distinction is valuable.

### 3. PASSING keyword for block syntax (§10)

Block syntax uses `PASSING name AS (...)` instead of `WITH name AS (...)` or curly braces.

**Rationale:** `WITH` collides with SQL's CTE syntax — a function call followed by `WITH` was ambiguous, requiring the parser to know function parameter names (coupling parser to type checker). `PASSING` is unambiguous (the parser distinguishes it without function metadata), has SQL/XML precedent (`XMLTABLE ... PASSING ...`), and reads naturally: "call this function, passing these fragments as parameters." Curly braces were rejected as too foreign to SQL.

### 4. No-overlap rule replaces union contexts (§6)

Parameter contexts must have unique column names. Union contexts (`Expr<Boolean, source | customers>`) are removed.

**Rationale:** Union contexts reintroduced SQL's join ambiguity problem — two tables with an `id` column required complex disambiguation rules that were never fully specified. The no-overlap rule eliminates this by construction. When multiple tables need to be accessible, authors use one of three strategies: (1) explicit SELECT into a CTE with unambiguous aliases, (2) typed `TableExpr` parameter requiring the caller to pre-join, or (3) `smelt.as_struct(alias EXCEPT ...)` for compile-time struct namespacing with zero runtime cost.

### 5. Context inference from splice points (§6)

The compiler infers a parameter's context from where it is spliced in the function body. When a parameter is used in multiple places, its context is the **intersection** of the schemas at each splice point — only columns with the same name and compatible type in all locations are available. Explicit context annotations are optional (for documentation and validation).

**Rationale:** Requiring explicit context annotations on every fragment parameter would be a significant annotation burden with diminishing returns — most parameters are used in exactly one place. Inference from the splice point gives the compiler the same information automatically. The intersection rule for multi-splice parameters is the safe default: the parameter can only reference columns guaranteed to exist everywhere it is used. Explicit annotations remain available for documentation and for narrowing the context further.

### 6. Multiple defines per file; smelt.metric() out of scope (§3)

A `.sql` file may contain multiple `smelt.define` definitions. `smelt.metric()` is independent from functions and not addressed by this design.

**Rationale:** Multiple defines per file is consistent with how models already work — a file is a compilation unit, not a one-definition container. `smelt.metric()` doesn't work today and has different design constraints (it's a semantic layer concept, not a composition mechanism); conflating the two would complicate both designs.

### 7. Bare columns from TableExpr allowed when unambiguous (§7)

Bare column references from `TableExpr` parameters resolve through standard SQL column resolution when no parameter name matches. Qualification is required only when a bare column name overlaps with a parameter name.

**Rationale:** Consistent with the no-overlap rule (§6) and parameters-first scoping. SQL developers expect bare column names to work — requiring qualification everywhere would be hostile. The parameters-first rule means parameters shadow columns, and the shadow warning catches accidental collisions. Qualification is the author's escape hatch, not a default burden.

### 8. WindowExpr\<T\> sort with linear subtyping (§2)

`WindowExpr<T>` is added as a third expression-level sort. The three sorts form a linear subtyping chain: `Expr<T> <: AggExpr<T> <: WindowExpr<T>`.

**Rationale:** Window functions in WHERE is one of the most common SQL errors. Neither `Expr<T>` nor `AggExpr<T>` captures "contains a window function." The linear chain makes the sort system enforce SQL's actual restriction rules naturally: a window context accepts all three, an aggregate context accepts `Expr` and `AggExpr` but rejects `WindowExpr`, and a scalar context accepts only `Expr`. No branching in the subtype relationship — just a line.

### 9. Numeric type system: linear promotion chain (§9, §13)

`Numeric` is a **type constraint** (not a concrete type), meaning "any type in the numeric family." The concrete numeric types form a linear promotion chain for computing the least upper bound (LUB) when types mix:

```
SmallInt < Integer < BigInt < Double
```

- `Float` maps to `Double` everywhere (not a distinct point in the chain).
- `Decimal` sits between `BigInt` and `Double` in the chain (`SmallInt < Integer < BigInt < Decimal < Double`) but **Decimal precision/scale tracking is deferred** to post-v1. In v1, `Decimal` is treated as a single type without precision parameters.

**Type constraints vs. concrete types:**
- `Expr<Numeric>` accepts `Expr<Integer>`, `Expr<BigInt>`, `Expr<Double>`, etc. — this is constraint satisfaction, not coercion.
- `Expr<Double>` does NOT accept `Expr<Integer>` — these are different concrete types. The user writes `CAST(x AS DOUBLE)`.
- Built-in operators and multi-argument functions (COALESCE, GREATEST, `+`, `-`, etc.) compute the LUB of their argument types using the promotion chain. `COALESCE(Integer, BigInt)` returns `BigInt`. `Integer + Double` returns `Double`.

**Built-in function return types use a canonical mapping table**, not generics:

| Function | Integer family input | Double input | Pattern |
|----------|---------------------|--------------|---------|
| SUM | BigInt | Double | Type-dependent return |
| AVG | Double | Double | Type-dependent return |
| MIN / MAX | same type T | same type T | Type-preserving (generic) |
| COUNT | BigInt (always) | BigInt | Fixed return |
| ABS | same type T | same type T | Type-preserving (generic) |
| SQRT, LOG, LN | Double (always) | Double | Fixed return |

Type-preserving functions use generics: `MIN<T: Ordered>(T) → T`. Type-dependent and fixed-return functions use hard-coded mappings in the canonical registry. This is consistent with the canonical CAST enforcement decision (§13) — smelt says `SUM(Integer) → BigInt`, enforces it with CAST, and if the user wants engine-native precision they use the backend namespace.

**Deferred:**
- Decimal precision/scale tracking (`Decimal(10,2) + Decimal(10,2) → Decimal(11,2)`). This is a refinement within the Decimal type, not a structural change to the type system. Can be added later without breaking anything.
- `Ordered` constraint specified in §16 decision 13.

**Rationale:** The linear chain matches what SQL engines actually do for the integer and float families. The hard problem — Decimal precision arithmetic — is orthogonal to the chain and can be deferred without affecting the rest of the type system. Treating `Numeric` as a constraint rather than a concrete type preserves the input type through generic functions (MIN returns Integer if given Integer, not an abstract Numeric), which is essential for the canonical CAST enforcement to produce precise output schemas.

### 10. Nullability tracking deferred

Nullability (`Expr<Integer NOT NULL>` vs. `Expr<Integer>`) is not tracked in v1. All expressions are implicitly nullable.

**Rationale:** Nullability tracking is valuable but orthogonal to the fragment sort system and numeric tower. Adding it later is non-breaking — it refines existing types rather than changing them. The main cost of deferral is that function return types can't distinguish "definitely not null" from "might be null," which means the LSP can't show nullability on hover. This is acceptable for v1 given the other priorities.

### 11. `smelt.define` grammar (§3, §21)

A `smelt.define` declaration is a **top-level statement** in a `.sql` file with this shape:

```
smelt.define <name>(<param-list>) [-> <return-type>] AS (<body>) [;]
```

- **Parameter list:** balanced `(...)`. Each param is `name [: <Type>] [= <default>]`. Trailing commas allowed.
- **Return arrow:** optional `-> <Type>` (present only in Tier 3, per §8).
- **Body marker:** required `AS` keyword (case-insensitive, matching SQL keyword convention).
- **Body:** balanced `(...)` containing one SQL expression or one SELECT statement. The outer parens are required — they make termination unambiguous without lookahead into SQL.
- **Terminator:** `;` is optional. The closing `)` of the body already terminates the declaration.

**File structure.** A `.sql` file is parsed as a sequence of top-level items: optional frontmatter (at most one, must be first), zero or more `smelt.define` declarations, and at most one bare model `SELECT`. Items are separated by whitespace only; no separator token required. Defines and a model `SELECT` may interleave freely.

**Frontmatter interaction.** *Superseded April 20, 2026 by §16 decision 22.* The original rule was "frontmatter applies to the file's model `SELECT` only; it does not apply to `smelt.define` bodies." Decision 22 generalises this: a frontmatter block attaches to the immediately following declaration (model, `smelt.define`, or `smelt.extern`), and each declaration may carry its own optional frontmatter. The rest of decision 11 — file-structure rules, error-recovery sync tokens, top-level-only nesting — is unchanged.

**Nesting.** `smelt.define` may not appear inside a SELECT, CTE, or another function body. Local/nested defines are not part of v1 and can be added later without breaking changes.

**Error recovery.** `smelt.define` is a top-level-only keyword path, which makes it a safe resync token. Recovery cases:
- Malformed parameter list: emit diagnostic, synthesize end of list at the next `AS` or `AS`-after-newline, continue to body.
- Missing `AS`: emit diagnostic, treat the next `(` as the body opener.
- Errors inside body `(...)`: standard Rowan SQL error recovery.
- Unrecoverable: skip tokens until the next top-level `smelt.define`, the frontmatter fence `---`, or EOF.

**Rationale:** The parenthesized body is the key decision. It gives the parser single-token termination (scan forward to balance parens) and eliminates ambiguity when multiple defines interleave with a model `SELECT` in one file. The alternative — ending a body at `;`, EOF, or the next `smelt.define` — creates real ambiguity and fragile error recovery for a minor cosmetic win. Every example in the paper already uses `AS (...)`, so this ratifies the surface syntax rather than changing it. Statement-only (not expression) keeps Tier 1 expansion straightforward: each call site expands against a fixed top-level declaration, with no scope-capture question for nested defines.

### 12. Expansion mechanics: AST-level with structured provenance (§8, §9, §12)

Function expansion operates on the CST, not on source text. The compiler clones the callee's body, substitutes argument subtrees into parameter placeholder nodes, and attaches a provenance trace to every resulting node. Expansion is **lazy**: calls remain symbolic through Level 1 planning and materialize at Level 2. Tier 1 type checking does not require materializing an expanded CST at all — it binds parameter names to argument types in the type context and checks the body under that context.

**Two senses of "expansion", different mechanisms.**

| Use | Trigger | Mechanism |
|---|---|---|
| Type-check expansion (Tier 1) | Checking a call to an unannotated function | Bind parameter names to argument types in the type context and re-check the body. No AST rewrite. |
| Codegen expansion (all tiers) | Level 2 strategy lowering | Clone the body CST, substitute arguments, attach provenance. Produces SQL. |

Tier 2/3 calls are never expanded for type checking — the declared signature is sufficient.

**Provenance trace.** Each node in an expanded CST carries an origin tag:

- `Caller(span)` — node came from the caller (argument subtree or surrounding call site).
- `Callee(fn_id, span)` — node came from the callee's body.
- `Synthesized(fn_id, reason)` — generated by the compiler (row-variable erasure, default-value insertion, strategy injection).

Nested calls push frames: `ExpansionFrame { callee_fn_id, callsite_span, param_bindings: [(param_name, arg_span)] }`. A → B → C produces a frame stack that the diagnostic reporter walks to render "in expansion of `B`, parameter `x` was bound to …" at any depth. The frame stack also answers the Tier 1 error-tracing data-structure question raised in §21.

**Hygiene.** The parameters-first rule (§7) is resolved at type-check time via the type context, not via token-level rewriting. At codegen time, parameter placeholders are already distinct CST node kinds, so there is nothing to collide with. CTE names introduced inside a function body can still collide with caller CTE names structurally — v1 emits a diagnostic on collision; v2 alpha-renames at expansion. Alpha-rename is mechanical once expansion is AST-level.

**Rationale.** Textual substitution was rejected for four reasons:

1. §8's Tier 1 error contract requires mapping errors back through expansion with parameter bindings. That is a source map — which a textual implementation would have to build anyway — at which point AST-level is strictly simpler.
2. Row-variable erasure (§11) synthesizes new field-access nodes, and default values (§21) inject fragments into parameter positions. Both construct AST; textual expansion would have to reparse its own output.
3. The parameters-first scoping rule (§7) is an AST-level semantic. Resolving it from tokens requires rebuilding scope information the CST already carries.
4. smelt-parser emits a lossless CST with position tracking as its primary output. AST-level expansion reuses that; textual discards it and rebuilds source maps by hand.

Textual's only real advantage is prototype simplicity, and that advantage disappears the moment the Tier 1 error contract is enforced. AST-level is the biggest architectural fork in the pipeline, and it lands clearly on the structured side.

**Deferred.**
- CTE alpha-renaming at expansion (v1 uses a collision diagnostic).
- Expansion caching for repeated calls with identical argument shapes (performance tuning, not correctness).
- Span-based deduplication of errors reported against the same callee body from many call sites (diagnostic polish).

### 13. `Ordered` constraint membership (§9, §21)

`Ordered` is the type constraint that gates `MIN`, `MAX`, `GREATEST`, `LEAST`, comparison operators (`<`, `<=`, `>`, `>=`), and `ORDER BY`. A concrete type `T` satisfies `Ordered` iff every v1 backend supports a total order on `T` natively.

**Members in v1:**

- All `Numeric` types: `SmallInt`, `Integer`, `BigInt`, `Float`, `Double`, `Decimal`
- `Text` (= `Varchar`)
- `Date`, `Time`, `Timestamp` (and `TimestampTz` if it exists as a distinct concrete type — orthogonal decision)
- `Boolean` (`FALSE < TRUE` on DuckDB, Postgres, Spark)
- `Interval`
- `Binary` (= `Blob`) — lexicographic

**Non-members in v1:** `Struct`, `Array`, `Map`. Ordering semantics diverge across backends (Spark orders arrays lexicographically; Postgres does not order arrays by default; DuckDB orders structs only if every field is itself orderable). Excluding them keeps the constraint backend-portable. If added later, they become derived: `Array<T>: Ordered where T: Ordered`, `Struct<fs>: Ordered where every field in fs: Ordered`.

**Collation for `Text` is unspecified.** `Ordered` is a pure membership predicate; it does not pin down the collation used to compare strings. v1 uses each engine's default collation, which means string ordering is not guaranteed to agree across backends. Collation tracking is deferred as a separate typed property, analogous to nullability (decision 10) — it refines existing types rather than changing the constraint surface.

**Relationship to `Numeric`.** `Numeric ⊂ Ordered`. Whether this is expressed as constraint subsumption in the signature language or as duplicated enumeration is a signature-language question, deferred to the generics-syntax item in §21.

**Rationale.** The membership table matches what every v1 backend already supports natively, so `MIN`/`MAX`/`ORDER BY` over these types compiles to native SQL without casts. Adding a type to `Ordered` later is non-breaking (old programs keep compiling, new ones gain an option); removing one is breaking. When in doubt, a type is included. Composite types are excluded because their cross-backend semantics genuinely differ — including them would force smelt to either pick a winner (breaking two of three backends) or emit verbose lexicographic-comparison scaffolding.

**Deferred.**
- Collation as a tracked property of `Text`.
- Decimal precision/scale in comparisons (follows whatever decision 9 eventually lands for Decimal arithmetic).
- `Ordered` membership for composite types (`Array`, `Struct`, `Map`) once their element/field constraints are formalized.

### 14. Generics syntax and inference (§9, §21)

Built-in signatures and `smelt.extern` declarations may be polymorphic via angle-bracket type parameters on the function name:

```
MIN<T: Ordered>(T) → T
COALESCE<T: Ordered>(T, T, ...) → T
DATE_ADD<T: Temporal>(T, Interval) → T
ABS<T: Numeric>(T) → T
```

Multiple parameters are comma-separated (`<T: Ordered, U: Numeric>`). An unconstrained parameter is `<T>`. No SQL conflict arises because signatures are a declaration form, never parsed as expressions.

**Scope in v1.** Generics are available only in built-in signatures and `smelt.extern` declarations. `smelt.define` (user-defined functions) stays monomorphic in v1; authors who want polymorphism write overloads. Generic user functions open higher-rank, variance, and constraint-inference questions the tier system (§8) was designed to avoid, and are deferred.

**Inference algorithm.** For each type parameter `T` in a signature, the checker collects every position where `T` appears: argument positions, and — in checking mode — the expected return type from context. It then resolves `T` by a single rule per constraint class:

- If `T`'s constraint has a **promotion chain** (in v1, only `Numeric` — see decision 9), bind `T` to the LUB of the positions under that chain.
- Otherwise, every position must unify to the same concrete type (engine aliases like `Text`/`Varchar` treated as equal). A mismatch is a type error at the call site.

After binding, the checker discharges the declared constraint: if `T: Ordered` and inferred `T = Map<…, …>`, the call fails because `Map` is not in `Ordered` (decision 13).

**Multi-position consequences.**

- `MIN(revenue)` where `revenue: Expr<Decimal>` → positions `{Decimal}` → `T = Decimal` → return `Expr<Decimal>`.
- `COALESCE(int_col, bigint_col)` → positions `{Integer, BigInt}` → LUB = `BigInt` → return `Expr<BigInt>`.
- `COALESCE(date_col, timestamp_col)` → `Ordered` has no chain between `Date` and `Timestamp` → error, explicit CAST required.
- `DATE_ADD(ts, interval)` → `T` appears in arg 1 and return; `T = Timestamp`.

**Bidirectional interaction.** When the call occurs in checking mode and `T` appears in the return position, the expected type is added as an additional position for `T`. It participates in the LUB / unification step like any other position. Example: context expects `Expr<Double>`, call is `COALESCE(1, 2)` with integer literals; positions for `T` become `{Integer, Integer, Double}`; LUB under the numeric chain = `Double`; literals type-check as `Double`. If the expected type cannot be reconciled with the argument-derived binding, the error is local and shows both sides.

**Error surface.** Generic-inference errors point to the specific argument (or context) position that forced the inconsistent binding and include the accumulated history of `T`: "`T` inferred as `Integer` from arg 1 (line 5); cannot unify with `Text` at arg 2 (line 6)." No "unresolved type variable" or "constraint unsatisfied in the prelude" style messages — these fail the Tier 1 error contract (§8).

**Constraint subsumption.** `Numeric ⊂ Ordered` (and any other subsumption relations the signature language acquires) is expressed directly as declared constraints. A signature `f<T: Numeric>(T) → T` accepts any `Numeric` concrete; since every `Numeric` is also `Ordered`, the caller can pass it to any `<U: Ordered>` position. The subsumption check is structural and does not require the user to restate constraints.

**Variadic positions** (e.g. `COALESCE<T>(T, T, ...)`) are represented abstractly as "some number of positions for `T`"; the concrete mechanism depends on the variadics decision (still open in §21). The inference rule above references "all positions for `T`" without committing to how variadic arity is spelled, so decision 14 is stable under either outcome.

**Rationale.** The syntax choice is optimized for reader recognition: anyone familiar with Rust, TypeScript, Scala, or C# reads `MIN<T: Ordered>(T) → T` correctly on first sight. Constraining generics to signatures in v1 preserves the tier architecture's property that user functions never introduce global inference obligations. The LUB-vs-unification split (keyed on whether the constraint has a promotion chain) prevents ad-hoc promotions from sneaking into non-numeric type families: if we ever want `Date + Timestamp → Timestamp` promotion, it becomes a deliberate extension of decision 9, not an emergent consequence of `COALESCE`'s signature.

**Deferred.**
- Generics in `smelt.define` (user-defined polymorphic functions).
- Higher-kinded constraints (`Array<T>: Ordered where T: Ordered`) — depends on composite-type handling.
- Multi-parameter constraint relationships (e.g. `f<T, U>(T, U) where T: CoercibleTo<U>`). Not needed for built-ins in v1.
- Bounded LUB widening for user-declared families beyond `Numeric`.

### 15. Variadics in built-in signatures (§9, §21)

Built-in signatures and `smelt.extern` declarations may mark the final argument position as variadic with a trailing `...`. The minimum arity of the call is the number of required positions preceding the rest:

```
COALESCE<T: Ordered>(T, T...) → T         -- 1 or more args
GREATEST<T: Ordered>(T, T...) → T         -- 1 or more args
LEAST<T: Ordered>(T, T...) → T            -- 1 or more args
CONCAT(Text...) → Text                     -- 0 or more args
CONCAT_WS(Text, T, T...) → Text            -- 2 or more args (separator + at least one payload)
```

A variadic position expands to N positions for inference, one per actual argument, all sharing the same type parameter (if one is declared). Decision 14's inference rule applies unchanged: positions under a promotion-chain constraint take the LUB, positions under any other constraint must unify. `COALESCE(int_col, bigint_col, double_col)` therefore binds `T = Double` and returns `Expr<Double>`; `COALESCE(text_col, int_col)` fails because `Text` and `Integer` have no common position under `Ordered`.

**Restrictions.**

- **Argument positions only.** Variadic return types are not permitted — SQL functions return a single column per call.
- **Built-ins and `smelt.extern` only.** `smelt.define` stays monomorphic in v1 (decision 14) and also stays non-variadic. Users write fixed-arity wrappers where needed.
- **Positional only.** Arguments passed to a variadic position use positional syntax; `name => value` named-argument syntax does not apply to variadics.
- **At most one variadic per signature, in final position.** Forms like `f(T..., U, V...)` are not expressible; they are not needed for any v1 built-in.

**Zero-arg edge case.** If a type parameter `T` appears only in a variadic position and the caller supplies zero actual arguments, inference has no positions for `T` from the call. Two fallbacks apply, in order:

1. In checking mode, the expected return type contributes a position for any `T` that appears in the return; use it.
2. Otherwise, the call is rejected with an error local to the call site: "cannot infer type parameter `T` — variadic position received no arguments and no return type is expected from context."

In practice this case is rare because zero-or-more variadics typically use a concrete element type (`CONCAT(Text...) → Text`), not a type parameter. Signatures that combine a zero-lower-bound variadic with a free type parameter are a signature-design smell; the error at least localizes it rather than producing an unresolved-variable leak across the module.

**Interaction with `PASSING`.** The `PASSING` clauses from §10 are structured fragment passes, not variadic arguments. They attach to named parameters declared with fragment sorts (`SelectItems`, `Predicate`, etc.) and are orthogonal to `...` in the inline argument list. A signature may use both mechanisms, but they do not overlap syntactically.

**Rationale.** Trailing `...` with a leading-required-positions floor is the most compact way to express all of COALESCE-family, CONCAT-family, and fixed+variadic cases without a new syntactic device per arity class. Folding variadic positions into decision 14's "collect all positions for `T`" keeps the inference algorithm single-rule. Rejecting variadics in user-defined functions in v1 preserves the property that user code never introduces unbounded inference obligations, matching the tier architecture's design target.

**Deferred.**
- Explicit minimum-arity notation (`T...2+`). Leading required positions cover every built-in we've encountered; revisit only if a use case appears.
- Variadics in `smelt.define` — follows decision 14's deferral of user-defined generics.
- Heterogeneous variadic tuples (TypeScript-style `...args: [Text, Integer, Date]`). Not needed for SQL built-ins.
- Named-argument syntax for variadic positions. Keeps the positional/named boundary clean in v1.

### 16. Tier 1 error tracing: single-level traces in Step 1 (§8, §19, §21)

Step 1 of the experimentation roadmap ships Tier 1 error tracing with **single-level traces only** — call site → innermost error, with the parameter binding that triggered the failure. Nested-frame rendering (walking a frame stack of arbitrary depth to produce "in expansion of B, parameter x was bound to …" at every level) is added in Step 2 as diagnostic polish. The frame-stack data structure from decision 12 is built from day one; only the renderer is single-level.

**What ships in Step 1:**

- A frame stack is pushed on every call expansion, as specified by decision 12. One frame or many, the structure is the same.
- The diagnostic reporter reads only the innermost frame and the outermost call site. The rendered error shows the call site (outer), the failing subexpression (inner), and the parameter binding that produced the failure.
- For a single-level call (`safe_divide(x, y)` where `x` is wrong type), this is the full trace.
- For a nested call (A → B → C), the user sees the outermost call site in A and the type error inside C, with the parameter binding at the innermost frame. Intermediate frames (the call from A to B, from B to C) are not rendered. The error is still local and actionable; it just doesn't narrate the full path.

**What lands in Step 2:**

- Full frame-stack rendering: every frame contributes a "in expansion of `fn`, parameter `p` was bound to ..." line, call-site first.
- This upgrade is purely a change to the diagnostic reporter. The frame stack is already populated correctly in Step 1; Step 2 reads more of it.

**Rationale.** Step 1's canonical target is the `safe_divide` end-to-end example, which is one level deep. Single-level traces cover it completely and prove the Tier 1 error contract from §8 (errors map back through expansion with parameter bindings). The multi-level renderer is mechanical to add once the single-level renderer exists and the frame stack is populated, so deferring it to Step 2 is low-cost and low-risk. The alternative — holding Step 1 until the full nested renderer is built — delays validation of the fragment-sort expansion model on a diagnostic feature that has no bearing on whether the model works.

Shipping single-level first also generates real data on nesting depth before polishing the renderer: if `safe_divide`-style calls are the dominant Tier 1 pattern and deep nesting is rare, the Step 2 upgrade becomes a small win rather than the other way around.

**Deferred.**

- Full nested-frame rendering — lands in Step 2, using the frame stack already maintained in Step 1.
- Span-based deduplication of errors reported against the same callee body from many call sites — diagnostic polish, already flagged under decision 12.
- Rendering strategies for very deep expansions (truncation, collapse of repeated frames) — only relevant once nesting is common enough to need ergonomics.

### 17. Tier 2 calling Tier 1: inline expansion at the Tier 2 body check (§8, §20D, §21)

A Tier 2 function (parameters annotated, body checked in isolation) may call a Tier 1 function (unannotated). The Tier 1 callee's body is **expanded inline during the Tier 2 body check**, using the Tier 2 function's declared parameter types as the concrete argument types at that expansion site. Errors in the expansion are reported against the Tier 2 body, with the frame-stack trace from decisions 12 and 16 rooted at the Tier 2 call site.

**Why not the three options from §20D.**

- *(a) Tier 1 return as `Any`/unknown.* Breaks isolation in a way the Tier 2 author cannot see: a typo in the Tier 1 helper silently erodes the Tier 2 caller's type safety. The whole point of Tier 2 is that body errors are caught at definition time.
- *(b) Refuse the call.* Eliminates a real use case. Library-quality Tier 2/3 functions often want to call small unannotated helpers. Refusing breaks the gradual-typing thesis (§8): you'd have to annotate every callee before writing the caller.
- *(c) Require callees ≥ caller's tier.* TypeScript `--strict` at the function level. Migration-hostile: any existing Tier 1 helper becomes an obstacle the moment a Tier 2 function wants to call it.

**The mechanism.** Tier 2 body check already knows its own parameter types. At a Tier 1 call inside that body, the checker collects argument types from the Tier 2 context, then runs Tier 1's standard expand-and-check routine (decision 12, Tier 1 column of the expansion table) with those types as the call site's concrete types. The Tier 1 body is checked under a type context binding its parameter names to the Tier 2–supplied argument types. The result is a synthesized return type, which flows back into the Tier 2 body check at the call position.

Transitive Tier 1 → Tier 1 chains compose: every level's Tier 1 callee is expanded with types derived from the Tier 2 root. No recursion (guaranteed by §3's cycle rule), so expansion terminates.

**Signature stability is preserved.** Decision 17 does not change what Tier 2's callers see. A Tier 2 function exposes only its declared parameter and (in Tier 3) return types. If a Tier 1 callee's body is temporarily broken mid-edit, the Tier 2 body check fails — but the Tier 2 signature is unchanged, so Tier 2's callers continue to type-check. This matches the §8 LSP-stability contract and is why Tier 2/3 is the right tier for shared code even under tier mixing.

**Cost.** Tier 2 body check is no longer zero-expansion — it expands each Tier 1 callee it reaches. Expansion is bounded by the call graph and cached via Salsa. The cost is paid only when the Tier 2 author chose to call an unannotated helper; Tier 2 → Tier 2/3 calls remain signature-only, no expansion.

**Edge case: unconstrained TableExpr parameters.** If the Tier 2 function has a bare `TableExpr` parameter (no column annotations) and passes it into a Tier 1 callee that does structural column resolution, some checks cannot fire during the Tier 2 body check — the schema is unknown until the Tier 2 function's own call sites bind a concrete table. These checks defer to Tier 2's call-site expansion, the same way Tier 2's own structural references against a bare `TableExpr` defer (§7, §20B). Decision 17 does not introduce a new deferral class; it extends the existing one through Tier 1 callees.

**Upgrade story.** Upgrading a Tier 1 helper to Tier 2 does not break Tier 2 callers that were relying on decision 17: once the helper is Tier 2, its declared signature is used at the call site instead of expansion, which is strictly more information. The only way an upgrade breaks a caller is if the author annotates a parameter type that excludes something the caller was passing — the same risk as §20D's upgrade-path note and the deferred "Tier 1 → Tier 2 breaking changes" item in §21.

**Rationale.** Decision 17 reuses mechanisms already specified. Tier 1's expand-and-check routine exists (decision 12). The frame-stack error tracer exists (decisions 12, 16). The type-context-binding approach to Tier 1 checking does not require a materialized expanded CST. Treating a Tier 1 call inside a Tier 2 body as "just another Tier 1 call site, with concrete types from the surrounding body" is the simplest rule that preserves isolation as Tier 2 promises it (definition-time error surface, stable signature), honours the gradual-typing thesis (mixing tiers is allowed and natural), and doesn't introduce a new concept (no `Any`, no tier-ordering invariant, no refusal rule).

**Deferred.**

- Tier 1 → Tier 2 upgrade-path breaking-change story (kept as an open item in §21).
- Caching of Tier 1 expansion checks across multiple Tier 2 body re-checks — performance tuning, orthogonal to correctness.
- Diagnostic polish when a Tier 1 expansion error fires inside a Tier 2 body that is itself reached via a long Tier 3 → Tier 2 → Tier 1 chain; covered by the deep-expansion rendering item under decision 16.

### 18. `PASSING` is a context-sensitive keyword (§10, §21)

`PASSING` is reserved **only** at the syntactic position immediately following the `)` that closes a `smelt.fn.*` call or a call to a `smelt.define`-declared function. Everywhere else in the grammar — column names, aliases, CTE names, ordinary SQL identifiers — `PASSING` is a regular identifier and parses unchanged.

**Trigger rule.** The parser identifies a smelt function call by its namespace-prefixed path (`smelt.fn.<...>` or a user-defined function reachable through the same resolution). After the closing `)` of such a call, the parser peeks one token:

- If the next token is `PASSING`, it begins a block-clause sequence (`PASSING <name> AS (<body>)`, repeated). Clauses continue until the next token is no longer `PASSING`.
- Any other token: `)` has ended the call expression, and normal SQL parsing resumes with `PASSING` treated as an identifier if it appears.

The check is uniform in expression position and FROM position — `SELECT smelt.fn.foo(...) PASSING ...` and `SELECT * FROM smelt.fn.foo(...) PASSING ...` use the same rule.

**What the parser does not need.** No consultation with the type checker, no knowledge of the callee's parameter list, no awareness of which fragment-sort parameters accept `PASSING` clauses. Those are type-checker concerns and run after parsing. The parser produces a CST with `PASSING` clauses attached to the call node; the type checker validates names, sort compatibility, and binding during the usual call-site check.

**What is not covered.** PASSING does not attach to plain SQL function calls (`UPPER(...)`, `SUM(...)`), aggregates, or arbitrary parenthesized expressions. If an author writes `SELECT UPPER(x) PASSING y AS (...)`, the parser treats `PASSING` as an identifier following `UPPER(x)`, which produces a normal SQL syntax error downstream — the right outcome. Black box functions declared via `smelt.extern` do **not** receive `PASSING` clauses in v1; they have no fragment-sort parameters (see §5 — externs carry typed signatures over `Expr<T>` / `TableExpr`, not `SelectItems` or `OrderSpec`). If externs ever grow fragment-sort parameters, extending the trigger rule is mechanical.

**Rationale.** Reserving `PASSING` globally would break any schema with a `passing` column or alias — rare but present in real analytics codebases (audit flags, test-result tables). Context-sensitivity eliminates that risk at the cost of a one-token lookahead after `)`, which the parser already performs for other purposes (e.g., distinguishing `)` terminating a subquery from `)` terminating a CAST). The trigger is purely syntactic — a namespaced function-call path — so the parser stays independent of the type checker, preserving the same layering decision 3 achieved (§10: "the parser operates independently of the type checker"). This also matches how Rust, Kotlin, and Swift handle context-sensitive keywords (`dyn`, `async`, `where` in specific grammar slots) without making them global reserved words.

**Deferred.**

- Extending the trigger rule to `smelt.extern` calls if externs ever acquire fragment-sort parameters. Not in v1.
- Whether PASSING can attach to chained calls (`smelt.fn.a(...).b(...) PASSING ...`). Method chaining on function return values isn't in the surface syntax today; revisit if it lands.
- Pretty-printer / formatter conventions for PASSING clauses (line breaks, indentation). Style concern, not grammar.

### 19. `smelt.as_struct()` deferred to post-v1 (§6, §21)

`smelt.as_struct()` — introduced in §6 as Strategy 3 for exposing multiple joined tables to caller-provided fragments without column-name collisions — is **not in v1**. The no-overlap rule stands on Strategies 1 (explicit SELECT into a CTE with aliases) and 2 (typed `TableExpr<{...}>` parameter requiring the caller to pre-join).

**What ships in v1.** §6's Strategy 1 and Strategy 2 remain fully specified and are the v1 options for multi-join functions that need to expose columns to caller fragments. Strategy 1 is the SQL-native approach — verbose but universal and immediately recognisable to SQL authors. Strategy 2 pushes the disambiguation responsibility onto the caller, which is appropriate when the function author wants to require a specific shape.

**What defers.** The full `smelt.as_struct()` design: surface syntax, `EXCEPT` resolution (including nested-struct paths), per-backend struct-literal generation, splice-context rules for where the generated struct expression can appear, and the interaction with backends that lack native struct literals (Postgres composite types are the awkward case).

**Rationale.**

1. The motivating use case — multi-table joins with overlapping column names — is real but not universal. In the analytics functions likely to ship in early smelt projects, Strategy 1 covers it with mechanical SQL that every reviewer can parse at a glance.
2. Compile-time struct generation is not architecturally lightweight. It requires schema extraction from `TableExpr` positions (ties into the "hardest problem" from §20I when the context involves unconstrained `TableExpr` parameters), backend-specific emission (DuckDB `{'f': v}`, Spark `struct(v AS f)`, Postgres requires row constructors or composite-type declarations — which is not a drop-in substitute), and splice-position rules that interact with the fragment-sort system in ways that deserve dedicated analysis rather than being folded into Step 3.
3. `smelt.as_struct()` conceptually overlaps with struct-value row polymorphism (§11, Step 8 of the experimentation roadmap: `Expr<Struct<{ts: Timestamp, ..r}>>`). Both concepts produce a named-field view over column-like data; designing them together once both are concrete avoids a two-stage surface where the first mechanism shapes the second in ways we'd want to revisit. Deferring `as_struct` to a revisit alongside Step 8 lets the two land with a coherent story.
4. Deferral is purely additive. Adding `smelt.as_struct()` later introduces a new expression form; it does not invalidate any v1 program. Strategies 1 and 2 remain the idiomatic options even after `as_struct` lands; they do not become deprecated.

**Cost of deferral.** §6 Strategy 3 drops out of the v1 surface. Authors writing multi-join functions with overlapping column names in caller fragments must reach for Strategy 1 or Strategy 2. The paper's §6 example that uses `smelt.as_struct(source EXCEPT customer_id, product_id)` is retained as an illustration of a future capability; v1 readers should treat Strategy 3 as "design sketch, deferred" rather than "available today".

**What a minimal v1 would look like (rejected).** A `smelt.as_struct(source)` form without `EXCEPT` was considered and rejected. `EXCEPT` is the reason the feature exists — join keys must be excluded so the remaining columns form a clean namespace. A variant that cannot exclude fields does not solve the motivating use case; it would ship syntax that serves no purpose until the full form arrives.

**When to revisit.** Coupled with Step 8 of the experimentation roadmap (struct row polymorphism). When Step 8 concretises the `Struct<{..r}>` surface, re-open `smelt.as_struct()` with the Step 8 vocabulary in hand and design them together.

**Deferred.**

- The full surface syntax: `smelt.as_struct(<table-expr> [EXCEPT <col-list>])` is the working sketch; confirm at revisit.
- `EXCEPT` semantics over nested struct columns (`EXCEPT event.meta.user_id`).
- Per-backend emission and the question of whether to support backends that lack struct literals via synthesised composite-type scaffolding or to emit a backend-capability error.
- Splice-context rules: can `smelt.as_struct()` appear in `SelectItems`? In an `Expr` position as a struct-typed scalar? Both, with constraints?
- Interaction with caller-provided fragments that reference synthesised struct fields — specifically, whether the type checker validates field references inside caller fragments at definition time or defers to call-site expansion.

### 20. Default value expansion for fragment sorts (§3, §21)

Defaults are attached to parameters by writing `= <fragment>` in the signature. The rules for how they are declared, checked, and expanded are as follows.

**Explicit defaults only — no implicit empty.** A parameter without a `=` clause is required; the caller must supply an argument. List-valued fragment sorts (`SelectItems`, `OrderSpec`) do **not** acquire an implicit empty default; an author who wants "splice nothing" writes `= ()` explicitly. This keeps the signature self-documenting — the reader sees either "required" or "default is `X`" and never has to reason about sort-specific implicit rules. It also matches the scalar sorts (`Expr<T>`, `TableExpr`, `AggExpr<T>`, `WindowExpr<T>`), which have no meaningful empty value and therefore always need an explicit default if one is intended. The surface syntax is uniform across sorts: explicitness at the signature, no hidden behaviour.

**Type-check at definition time.** Each default's CST is checked against the parameter's declared sort, and (in Tier 2/3) against its declared concrete type. Examples:

- `= TRUE` on `Expr<Boolean>` — literal checks as `Expr<Boolean>`.
- `= INTERVAL '30 minutes'` on `Expr<Interval>` — interval literal checks.
- `= smelt.ref('fallback_orders')` on `TableExpr` — ref resolves to a table, checks as `TableExpr`.
- `= ()` on `SelectItems<Agg, sessionized>` — empty list trivially satisfies any list sort, context-binding included.
- `= (COUNT(*) AS n)` on `SelectItems<Agg, sessionized>` — structural sort check at definition time (is it a list of aggregates?); column-name validation against the CTE context defers to call-site expansion, same deferral §6 flags for `source.*`-bearing CTE schemas.

Tier 1 functions have no declared parameter types, so the default's synthesised type becomes the parameter's type for any call site that omits that argument — the same mechanism Tier 1 uses for any other argument (decision 12's type-context binding).

**Expansion is call-resolution-time binding; CST cloning is at Level 2.** When a caller omits an argument, the parameter binding resolves to the default's declaration-site CST. No cloning happens at call resolution — the binding refers to the declaration-site subtree. At Level 2 materialisation (decision 12), the CST is cloned into the parameter's placeholder positions with `Synthesized(fn_id, "default for <param>")` provenance per decision 12's provenance model. This is identical to how an explicitly-passed argument is materialised, differing only in the provenance tag.

Tier 1 type checking does not materialise an expanded CST at all (decision 12), so defaults participate in the type-context binding: the parameter name binds to the default's type, and the body is re-checked under that context. No CST cloning, no extra work.

**List-splice comma elision.** List-valued fragment parameters can expand to zero elements (when the default or argument is `()`). The expander treats list-splice points as list-join nodes — adjacent commas surrounding a zero-element splice are elided. `SELECT id, name, metrics` with `metrics = ()` becomes `SELECT id, name`. This rule is general for list-valued fragment sorts (it applies to explicitly-passed `()` arguments just as it applies to defaults); defaults just make the empty case common. The rule is syntactic, runs at Level 2 materialisation, and does not consult the type checker.

**Self-containment (from §3) reaffirmed.** A default expression cannot reference other parameters. This keeps evaluation order trivial and means each default can be resolved independently at call resolution without solving a dependency graph among parameters. A caller that wants a default "relative to another argument" writes the relationship at the call site instead.

**Interaction with `PASSING` clauses.** A caller can omit a `PASSING` clause the same way as an inline argument — the parameter's default applies. No special handling in the parser or the type checker beyond what decision 18 already specifies: `PASSING` clauses attach to the call node at parse time, and defaults fill any parameter whose binding is still empty after both inline arguments and `PASSING` clauses are resolved.

**Rationale.** The bulk of the specification here is "defaults are just another source of argument fragments, pipe them through decision 12's machinery." The choices that required explicit decisions were (i) whether list sorts get an implicit empty default (no — explicitness over magic), (ii) where comma elision lives (Level 2, syntactic, general list-splice rule), and (iii) how context-bound defaults handle column-name validation (same call-site deferral §6 already uses). None of these introduce new mechanisms — they specify the boundary between existing ones.

**Deferred.**

- Column-name validation for non-empty context-bound defaults at definition time, once the CTE-schema extractor is call-site-agnostic enough to run without a concrete call (ties into §6's "Remaining Edge Cases" and §20B's forward-reference concern).
- Defaults that reference other parameters (`= b + 1` where `b` is a parameter). Deliberately excluded in v1; revisit only if a motivating use case appears and only after specifying the evaluation-order rule.
- Default values on row-polymorphic parameters (§11 already notes "No defaults on row-polymorphic parameters in v1"). Consistent with decision 20.
- Diagnostics for defaults that produce valid fragments but semantically-surprising SQL (e.g., `= TRUE` in an `OR` context, where `X OR TRUE` is always `TRUE`). Style linting, not correctness.

### 21. `smelt.extern` full syntax (§5, §13, §21)

`smelt.extern` declares a black-box function — a transformation where smelt knows the signature but cannot inspect the body. The declaration form and surrounding rules are as follows.

**Grammar.** `smelt.extern <name>(<param-list>) -> <return-type> [;]`. No `AS (...)` body — there is no body. Return type is mandatory. Externs are always "Tier 3 equivalent"; gradual typing (§8) does not apply. Bare-keyword `@annotation` syntax is not used — see decision 22.

**File placement.** Same rules as `smelt.define` from decision 11: any `.sql` file may contain any number of `smelt.extern` declarations, interleaved with `smelt.define` declarations, a model `SELECT`, and at-most-one file-level concept. `smelt.extern` is a safe resync token for parser error recovery, same status as `smelt.define`.

**Per-declaration frontmatter.** An optional YAML frontmatter block (`---` ... `---`) may immediately precede a `smelt.extern` declaration, carrying structured metadata — primarily per-backend emission rules, plus any properties that elsewhere would have been annotations (see decision 22). Shape:

```yaml
---
deterministic: true
idempotent: true
backends:
  duckdb:
    emit: read_parquet
  spark:
    emit: spark_read_parquet
---
smelt.extern read_parquet(path: Expr<Text>) -> TableExpr
```

The declared name (`read_parquet`) is the smelt-facing name — what callers write in model bodies. Each backend's emission is controlled by its `emit` entry in frontmatter. If `backends` is absent, the default is `backends: all` with `emit` equal to the declared name verbatim.

**Call surface is bare-name, not via `smelt.fn.*`.** Calling `read_parquet(x)` in a model body compiles to whatever the active backend's `emit` specifies — for DuckDB, `read_parquet(x)`; for Spark, `spark_read_parquet(x)`. This is consistent with §5's "the function name passes through to the generated SQL" principle, generalised so the passed-through name can differ per backend.

**Backend namespace is sugar.** `smelt.extern duckdb.read_parquet(...)` is equivalent to declaring `smelt.extern read_parquet(...)` with frontmatter `backends: { duckdb: { emit: read_parquet } }`. Authors can use whichever form is clearer: the namespace prefix for simple "engine-native name, one backend" cases; explicit frontmatter for "different emitted name per backend" or more complex configuration. Namespaced declarations may still carry frontmatter for other properties; the namespace is shorthand for the backend-emission subset only.

**Type-checker treatment.** Identical to SQL built-ins in the canonical registry (§13). The checker looks up externs by name in a namespace shared with built-ins, verifies argument types against the signature, and treats each call as an atomic planner node. The only difference between an extern and a built-in is provenance (shipped vs. user-declared), which is invisible to the caller and the type checker.

**Name collisions with the canonical registry.** An extern's smelt-facing name cannot collide with any entry in the canonical built-in registry. Shadowing a built-in with a user extern invites silent semantic divergence; a declaration-time diagnostic rejects it. Emitted names (in frontmatter) are the author's problem — smelt does not verify that the emitted name exists on the target backend (that's a runtime error at query execution time).

**Multiple externs per file.** Permitted. Each carries its own optional preceding frontmatter. Frontmatter placement follows the general rule from decision 22: a frontmatter block attaches to the immediately following declaration.

**Rationale.** Most of the specification reuses decisions already made: decision 11's file-structure rules extend verbatim; decision 13's canonical registry treatment applies identically to externs; decision 22's frontmatter-over-annotations choice applies here too. The one novel piece — per-declaration frontmatter for backend-specific emission — replaces what earlier drafts showed as `@backends(...)` annotations. Decoupling the declared (smelt-facing) name from the emitted (backend-facing) name is what lets one author-facing API dispatch to differently-named backend functions, which is common in practice (e.g., `read_parquet` vs. `spark.read.parquet`).

**Deferred.**

- Full schema for per-backend emission (currently just `emit: <name>`). Future extensions might include argument-position rewriting, required-import declarations for Python UDFs, or emission templates with placeholders. None are needed for v1.
- Runtime schema validation (checking actual output of an extern against the declared return type on first execution) — noted in §20L as a soundness safety net. Valuable but post-v1.
- `smelt.extern` interaction with partial signatures or introspection-driven signature inference (noted as an open question in §18). Defer until a concrete use case appears.
- Cross-file name collision rules for externs declared in multiple files with the same name. The simplest rule — forbidden, declaration-time diagnostic — matches how smelt-define works (decision 11) and is assumed here; a more sophisticated scope mechanism can come later if needed.

### 22. Unified frontmatter; annotations removed (§3, §5, §6, §12, §13, §21)

All previously-proposed `@annotation` syntax (`@deterministic`, `@idempotent`, `@append_only`, `@backends(...)`, and the deferred `@joins(...)` / `@provenance(...)`) is **removed from the language**. Every property that would have been an annotation is now a frontmatter key. Frontmatter applies to all three declaration kinds — models, `smelt.define`, and `smelt.extern` — with a single placement rule.

**The placement rule.** A frontmatter block (`---` ... `---`) attaches to the declaration immediately following it. A file is a sequence of top-level items; each declaration may optionally be preceded by a frontmatter block carrying that declaration's metadata. There is no file-level frontmatter scope separate from the declarations; frontmatter is always "the next declaration's frontmatter."

Example — one file with a frontmatter-bearing model and a frontmatter-bearing function:

```sql
---
materialization: table
---
SELECT * FROM smelt.ref('orders');

---
deterministic: true
---
smelt.define safe_divide(
    numerator: Expr<Numeric>,
    denominator: Expr<Numeric>
) -> Expr<Double> AS (
    CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL
         ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
    END
)
```

**Supersedes decision 11's frontmatter rule.** Decision 11 said "frontmatter applies to the file's model `SELECT` only." That restriction is lifted. Decision 11's file-structure rules (multiple items per file, optional trailing `;`, `smelt.define` as resync token) remain in force; only the frontmatter-scoping clause is revised.

**Property-to-key mapping.** The annotations we had mentioned map to frontmatter keys as follows:

| Former annotation | Frontmatter key | Value shape |
|---|---|---|
| `@deterministic` | `deterministic` | `true` / `false` (default `false`) |
| `@idempotent` | `idempotent` | `true` / `false` (default `false`) |
| `@append_only` | `append_only` | `true` / `false` (default `false`) |
| `@backends(all)` | `backends` | `all` (string) or list of backend names |
| `@backends(duckdb, postgres)` | `backends` | `[duckdb, postgres]` |
| `@joins(...)` (deferred) | `joins` (deferred) | Structured map (shape TBD when it lands) |
| `@provenance(...)` (deferred) | `provenance` (deferred) | Structured map (shape TBD) |

**Rationale.**

1. **Single mechanism.** Smelt already parses YAML frontmatter for model configuration. Introducing `@annotation` syntax adds a second mechanism carrying the same class of data. Consolidating to frontmatter removes the second parser, the grammar question of where annotations attach, and the "which syntax do I use for this property?" cognitive cost.
2. **Structured data works naturally.** `backends: { duckdb: { emit: read_parquet } }` (from decision 21) is awkward to spell in annotation syntax without inventing a structured-argument grammar that duplicates YAML. Frontmatter handles it directly.
3. **Separation of concerns.** The signature line answers "what are the types and sorts?" Metadata answers "how does this behave?" Keeping them syntactically separate is a readability win once declarations carry more than one property.
4. **Migration-friendly for deferred properties.** When `joins` / `provenance` land, they're just new YAML keys. In the annotation model, they'd require grammar extensions for structured arguments — a source of churn we avoid by starting uniformly.
5. **Annotations can return later as pure sugar.** If ergonomic evidence shows inline `@deterministic` is genuinely better for single-property declarations, annotations can be added back as literal desugaring into frontmatter keys. Decision 22 keeps that door open — starting with only frontmatter means adding annotations later is additive, not a redesign.

**Cost acknowledged.** A declaration with a single simple property (e.g., `deterministic: true`) costs three lines of YAML framing one line of content, versus an inline `@deterministic` of zero overhead lines. For files with many one-property declarations, this is visibly noisier. The trade is accepted because declarations are written once and called many times, the structured-data case dominates as more properties land, and the "one mechanism" win is large.

**Built-in registry note.** SQL built-ins in smelt's canonical registry (§13) are shipped with smelt, not written in user `.sql` files. Their properties (deterministic, backends, etc.) live in the registry's data format, not in `.sql` frontmatter. Decision 22 governs user-authored declarations — models, `smelt.define`, and `smelt.extern`. The canonical registry's internal format is orthogonal and unchanged by this decision.

**Deferred.**

- Full frontmatter schema per declaration kind (what's valid on a model vs. a `smelt.define` vs. a `smelt.extern`). Covered case-by-case as properties are specified; no single schema document in this paper.
- Annotations-as-sugar. Not in v1; revisit only if ergonomic evidence demands it.
- Validation of cross-cutting property relationships (e.g., a function with `deterministic: true` calling a function with `deterministic: false` — should that be a warning?). Property semantics beyond the basic "what does this mean?" question are planner-integration concerns, deferred to Step 7.

### 23. Engine-agnostic function bodies; backend specificity via namespace and `backends:` (§20G, §21)

**Decision.** Function bodies are written in smelt's **canonical SQL** — engine-agnostic by construction. Engine-specific features are reached only through the backend namespace (`duckdb.*`, `postgres.*`, `spark.*`). The `backends:` frontmatter property, introduced in decision 9 and carried by decision 22, is the sole mechanism for declaring engine restrictions; there is no per-function dialect tag.

**Rules.**

1. **Canonical-SQL bodies.** A function body with no backend-namespace references is portable: it type-checks against the canonical signature registry (§13) and emits on any supported backend. `INTERVAL '30 minutes'`, backend-specific cast syntaxes, and similar dialect idioms are *not* canonical — they must go through a canonical construct (e.g., `smelt.interval(30, "minutes")` in the canonical registry) or through a backend namespace call.
2. **Backend namespace is the escape hatch.** A function that genuinely needs DuckDB-only behavior calls `duckdb.read_parquet(...)` etc. The backend-namespace reference is a visible, typeable commitment — not a hidden dialect assumption in a string literal.
3. **`backends:` is inferred, narrow-only.** The planner infers a function's `backends` set as the intersection of the `backends` of every call in its body (backend-namespace calls contribute their single backend; canonical calls contribute the universal set). A declared `backends:` in frontmatter *narrows* inference — it may remove backends the body supports, but may not add backends the body does not support. Widening is an error.
4. **No dialect tag on functions.** A function is not "a DuckDB function" or "a Spark function." Its backend compatibility is a derived property, visible in tooling, not a sort or a declared attribute beyond the narrowing `backends:` set.
5. **Translation happens once, at final expansion.** After Level 3 lowering (§12), the canonical SQL tree is handed to the chosen backend's printer, which translates canonical constructs to that backend's surface syntax. Functions don't translate themselves; they produce canonical SQL, and translation is a single pass over the fully-expanded tree.

**What "canonical SQL" means, pragmatically.** Canonical SQL is whatever smelt's translation layer can emit faithfully on every supported backend. The set grows as the translation layer matures — early versions may restrict canonical SQL to a conservative common subset and reach into backend namespaces for anything non-portable; over time, more constructs migrate into canonical as translation rules are written. This is intentionally a moving, implementation-driven boundary rather than a fixed spec: it tracks the engineering reality of "what do we actually know how to translate?"

**Rationale.**

1. **Composes with decision 22's `backends:` inference.** A body that mentions only canonical calls and `duckdb.read_parquet` has inferred `backends: [duckdb]`. A body using only canonical calls has the universal set. The rule falls out of frontmatter semantics — no extra machinery.
2. **Visibility.** An `INTERVAL '30 minutes'` string literal hides its DuckDB-ness from the type system. A `duckdb.interval(...)` call surfaces the commitment where the planner, LSP, and reader can all see it.
3. **Avoids the "function is a DuckDB function" framing.** Tagging functions by dialect bifurcates the library ecosystem — users wonder whether to write a DuckDB version and a Spark version of the same logic. Canonical-by-default keeps the library single-sourced; specialization happens only when a backend genuinely has a feature no other backend does.
4. **Narrow-only declared `backends`** prevents the declaration from lying about portability. If the body calls `duckdb.read_parquet`, declaring `backends: [duckdb, spark]` doesn't make it run on Spark — the inference ceiling bounds what the declaration can claim.
5. **Single translation pass** matches the existing planner shape (§12): expansion is a tree rewrite in canonical form; backend printing is a separate downstream phase. No per-function translation state.

**Deferred.**

- Full enumeration of canonical SQL constructs. Which date/time functions, which interval spellings, which cast forms are canonical is a translation-layer implementation concern, specified as translation rules are written, not in this paper.
- Warning policy when an inferred `backends` set collapses to empty (i.e., a body mixing calls from incompatible backend namespaces). Likely a hard error, but the diagnostic design lives with the planner work.
- Whether `backends` affects callers transitively via a join/meet rule, and how cross-backend calls are blocked or bridged. Tied to the exchange/materialization story, already outside this paper's scope.

### 24. SelectItems\<Kind, ctx\> parallel to the expression chain (§2, §20A)

`SelectItems<K, ctx>` carries a kind parameter `K ∈ {Scalar, Agg, Window}` that is the ceiling of the expression sorts in the list. The three kinds form a linear subtyping chain that mirrors the expression chain: `SelectItems<Scalar, ctx> <: SelectItems<Agg, ctx> <: SelectItems<Window, ctx>`.

**Rationale.** Without a kind parameter, a `SelectItems` carrying a `WindowExpr` could be spliced into a `GROUP BY` query and the sort system would silently lose the restriction — the same class of error `WindowExpr<T>` was introduced (decision 8) to prevent at the expression level. The kind parameter extends that protection across the list boundary. Using the ceiling (not a per-element tag, not a uniform kind) matches SQL's reality — real `SELECT` clauses mix scalars and aggregates under a single `GROUP BY` — without adding structure beyond what the expression chain already provides.

**Splice-point rules.**

1. A plain `SELECT ${items} FROM t` with no `GROUP BY` and no other aggregates accepts `SelectItems<Window, ctx>` — any kind.
2. `SELECT ${items} FROM t GROUP BY ...` accepts `SelectItems<Agg, ctx>` and rejects `<Window, ctx>`.
3. A position restricted to pure scalars (rare in practice) accepts only `<Scalar, ctx>`.
4. The implicit-single-group case (a SELECT with no `GROUP BY` whose items are all aggregated) fits naturally: the items have ceiling `Agg`, the splice point accepts `<Window, ctx>`, and the subtyping check passes. This is the same SQL shape that §2 adds to the aggregate-context definition.

**Mixed lists.** A list mixing `user_id` (scalar) and `COUNT(*) AS n` (aggregate) has kind `Agg`. The author does not tag items; the kind is computed from the contents.

**Context binding** (the `ctx` parameter) is orthogonal to `K` and follows §6 unchanged: column-name validation against a CTE context that includes `source.*` still defers to call-site expansion.

**Resolves.** §20A "`SelectItems` is under-specified" — the `Agg` kind parameter now has a formal definition (ceiling of contained expression sorts), a subtype relationship to `Scalar` and `Window`, and a specified answer for mixed lists.

## 17. Comparisons and Theoretical Foundations

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
| **F#** | One-pass left-to-right HM with deliberate limitations | Validates "limit inference for better errors": F# spec constrains type variables specifically to simplify error messages. Type abbreviation preservation in errors as a spec-level requirement. |
| **Ermine (Kmett, S&P)** | Haskell-like with row types + kind polymorphism for financial reporting | Validates row polymorphism for typed SQL/relational composition at industrial scale (in production since 2008). Shipped without row constraints -- smelt's annotations are more ambitious. `Loc` annotations in type AST for error precision. |

### Theoretical Underpinnings

The design draws from several established PL techniques:

- **Fragment sorts / syntactic categories.** Multi-sorted algebras; Rust `macro_rules!` fragment specifiers. The foundation of safe composition.
- **Compile-time code generation.** MetaML (Taha & Sheard); Terra. Inspiration for typed code generation, though smelt uses single-phase expansion rather than multi-stage programming.
- **Hygienic macro expansion.** Kohlbecker et al., 1986; Clinger & Rees, 1991. Lexical scoping for parameters prevents C-preprocessor-style surprises. smelt's hygiene is closer to Rust's expansion-context model than to Scheme's full hygiene.
- **Row polymorphism.** Wand, 1987 (original formulation); Remy, 1994 (extension to ML); OCaml object types; PureScript row types. Structural column/field resolution for tables and structs.
- **Gradual typing.** Siek & Taha, 2006. Optional annotations with a clean adoption trajectory.
- **Totality via structural restriction.** Turner, 2004. No recursion guarantees termination.
- **Enriched type annotations.** Conceptually related to refinement types (Rondon et al., 2008), though the mechanism is explicit annotation rather than solver-verified logical predicates. Function types carry structural metadata (provenance, join graphs) beyond the basic sort.
- **Bidirectional type checking.** Pierce & Turner, 2000; Dunfield & Krishnaswami, 2021. Types flow up (synthesis) and down (checking).
- **Progressive lowering.** MLIR. Three planner levels with clear contracts at each boundary.
- **Polymorphic type inference for relational algebra.** Buneman et al. (1996); Van den Bussche & Waller (2002). Standard HM cannot handle relational algebra operations -- the relationship between a record type and its fields requires row type extensions. smelt's bidirectional checking with local row unification is a pragmatic simplification that works because smelt functions are first-order.

### Historical Precedents Worth Studying

- **SML Functors / OCaml modules** -- Parameterized modules that produce types based on input types. Context bindings like `Expr<T, source>` are a simplified version.
- **Scala's path-dependent types** -- `source.Column` where the type depends on a specific value. Context bindings are more constrained, avoiding Scala's complexity.
- **Template Haskell** -- Multi-stage compilation where generated code is type-checked after splicing. Tier 1 is exactly this. TH showed that error messages are the main usability challenge.
- **Liquid types (Rondon et al., 2008)** -- Refinement types carrying logical predicates. The compiler-derived structural metadata is a domain-specific form.
- **Heeren et al. (2003), "Top Quality Type Error Messages"** -- Analysis of why constraint-based systems produce poor errors. A "what to avoid" reference justifying bidirectional checking.
- **Ermine (Kmett et al., 2008-2013)** -- A lazy, pure Haskell-like language with rank-N types, kind polymorphism, and row polymorphism, built at S&P Capital IQ for financial report generation. Reports are specified via relational algebra combinators that compile to SQL. Production validation that row-polymorphic types over relational data work at scale. Notably, Ermine shipped without row constraints -- structural matching by field name/type was sufficient for a decade of financial reporting. The type AST includes `Loc` annotations for source positions, ensuring every type node carries its origin for precise error reporting.
- **F# type system design** -- F# deliberately constrains its type inference (one-pass, left-to-right) to improve error message quality. The F# spec mandates that "implementations should attempt to preserve type abbreviations when reporting types and errors" -- errors say `UserId`, not `int`. The spec also restricts type variable constraints specifically "to simplify type inference, reduce the size of types shown to users, and help ensure the reporting of useful error messages." This validates smelt's bidirectional checking choice: limiting inference power for error locality is a proven strategy.

## 18. Open Questions

### Specification to Tighten

- ~~**Union context disambiguation.**~~ **Resolved (April 18, 2026):** Union contexts replaced by no-overlap rule. See §6.
- **CTE context checking boundary.** `SelectItems<Agg, sessionized>` can be structurally checked at definition time, but column-name validation is call-site-dependent when the CTE includes `source.*`. The split needs clear documentation.
- **`AggExpr<T>` -- keep or collapse into `Expr<T>`?** Same argument as the Predicate removal: aggregation context is enforced by SQL syntax. Counter-argument: "this parameter expects an aggregate" is a common source of confusion. Deferred to implementation -- not in MVP scope either way.

### Deferred

- **LSP block-context completion** -- architecturally hard, can land after basic diagnostics.
- **Function tests** -- functions remain testable through models that use them. First-class function-test workflow is a follow-up.
- **Package ecosystem / registry** -- not in initial scope.
- **Python model interaction** -- functions are SQL-only; Python models are opaque table producers reachable via `smelt.ref()`.
- **Structured frontmatter properties** (`joins`, `provenance`) -- deferred until the planner needs them. Per §16 decision 22 these live in frontmatter alongside the simple boolean properties.
- **Error trace depth for nested calls** -- when A calls B calls C and C errors, Tier 1 shows A->C (call site -> innermost error), skipping intermediates. Full-chain traces are a future improvement.

### Unified Model

- **`smelt.extern` interaction with gradual typing.** Black box functions are always fully annotated, but what about partial signatures? Can a UDF declaration omit the return type and have it inferred from engine introspection?
- ~~**Engine-specific overloads for black box signatures.** `SUM` returns `HUGEINT` in DuckDB but `DECIMAL` in Spark. Should the signature registry support per-dialect overloads, or normalize to a common supertype?~~ **Resolved (April 18, 2026):** One canonical registry with canonical return types enforced via CAST. Backend-native precision available through backend namespace (`postgres.sum()`). See §13.
- **Parameterized model syntax.** How does a caller override a model's DAG-default refs? Syntax for call-site binding of model parameters needs specification. Related: how does this interact with the scheduler (a parameterized model may produce multiple outputs)?
- **Python models in the unified view.** Python models are currently opaque `TableExpr` producers. Under the unified model, they are black box materialized functions with no inspectable body and a schema inferred from execution. Does `smelt.extern` cover this, or does Python model integration need its own declaration mechanism?

## 19. Experimentation Roadmap -- What We Learn at Each Step

This is a research sequence, not a shipping plan. Each step teaches something that informs the next.

### Step 1: Fragment Sorts + Expr<T> Functions

**Build:** `smelt.define` for expression-level functions. `Expr<T>` sort. Tier 1 checking (expand, check, trace errors back). The `safe_divide` example end-to-end.

**What we learn:** Does the fragment sort concept work in practice? Does expansion + type checking produce errors good enough for Tier 1? Is the `smelt.define` / `smelt.fn.*` syntax natural?

**How it ladders:** If Tier 1 errors are adequate, the gradual typing thesis holds. If not, we know exactly where the pain is before adding complexity.

### Step 2: Black Box Signature Language + Built-in Function Typing

**Build:** Canonical signature registry for SQL built-ins (one registry, not per-dialect). The ~80% of built-ins with simple signatures (`Expr<T> -> Expr<T>`, `Expr<T> -> AggExpr<T>`). `smelt.extern` for user-declared black box functions. Generics/type parameters for the ~20% that need them (`COALESCE<T>`, `ARRAY_AGG<T>`). `backends` frontmatter property for portability tracking (per §16 decision 22). Backend namespace (`duckdb.*`, `postgres.*`) for engine-specific functions and native-precision opt-in. CAST enforcement for canonical return types.

**What we learn:** Can the fragment sort system extend to cover SQL built-ins? Does the signature language need variadics immediately, or can fixed-arity overloads suffice for MVP? Is the `smelt.extern` declaration natural for UDFs? What is the effort/value ratio of each signature language extension (generics, variadics, type-as-arguments)? Does the canonical-type-with-CAST approach produce correct schemas across backends? Is the backend namespace natural for opting into engine-specific behavior?

**How it ladders:** This is mandatory infrastructure -- every subsequent step benefits from built-in type information flowing through the checker. Generics here also inform the Tier 2 bidirectional checker (Step 5). Black box functions are simpler than transparent functions (no expansion, no body analysis), so this is a good early test of the signature language before applying it to the harder transparent case. The `backends` frontmatter property establishes the portability model that the planner needs for multi-backend execution.

### Step 3: TableExpr Functions + Row Polymorphism

**Build:** `TableExpr` sort. Structural column resolution (bare column names resolving against table schemas). The `sessionize` and `add_margin` examples.

**What we learn:** Does structural column resolution work? How do errors feel when a table is missing required columns? Is the hybrid scoping model (lexical parameters + structural columns) confusing or natural?

**How it ladders:** This validates the row polymorphism thesis. If structural resolution works for tables, the same concept extends to structs (Step 8).

### Step 4: Context Bindings

**Build:** Context parameters on fragment sorts. `SelectItems<Agg, sessionized>`, `Expr<Boolean, source>`. CTE-derived contexts. Union contexts.

**What we learn:** Can we derive CTE schemas reliably? Do context-bound errors actually help? Is the capability-based access control ("author controls what callers see") valuable in practice?

**How it ladders:** Context bindings are what make the error story qualitatively better than "just expand and check." This is the bridge from "it works" to "it works well."

### Step 5: Tier 2 Annotations + Bidirectional Checking

**Build:** Parameter type annotations. Bidirectional checking in synthesis and checking modes. Pre-expansion call-site checking. The generics from Step 2 must integrate with bidirectional checking here.

**What we learn:** Does pre-expansion checking produce meaningfully better errors? Is bidirectional checking sufficient, or do we hit cases that want constraint-based solving? How much annotation do people actually write?

**How it ladders:** This is the inflection point for library-quality functions. If Tier 2 errors are good, Tier 3 (return type annotations) is a small incremental step.

### Step 6: Block Syntax

**Build:** Trailing `PASSING name AS (...)` clauses on function calls. Parser integration and error recovery.

**What we learn:** Is the parser complexity manageable? Does the syntax actually improve readability over inline arguments? How does error recovery work inside blocks?

**How it ladders:** Block syntax is pure ergonomics. It does not change the type system. It can be added or deferred without affecting anything else. Can happen any time after Step 1.

### Step 7: Planner Visibility

**Build:** Functions as nodes in the logical plan. Explicit property annotations. Column provenance annotations. The join elimination example. The transparency rule: optimize across transparent boundaries, treat black box as atomic.

**What we learn:** Can planner rules reason about functions effectively? Does join elimination actually fire on real workloads? Is the explicit annotation burden acceptable, or is automatic derivation needed sooner than expected? Does the transparent/black box boundary give the planner a clean optimization rule?

**How it ladders:** This is where smelt functions become fundamentally different from Jinja macros. Everything before this is "better macros." This is "optimization annotations."

### Step 8: Struct Row Polymorphism

**Build:** `Expr<Struct<{ts: Timestamp, ..r}>>`. Row variables on struct types. Spread syntax. Value-level erasure at expansion.

**What we learn:** Do analytics teams actually have struct-typed columns that benefit from this? Does the single-named-variable restriction bite? Can row-unification errors be explained clearly?

**How it ladders:** Validates whether row polymorphism generalizes from tables to values. If it does, the type system has broader reach than initially designed for.

### Dependency Graph

```
Step 1 -> Step 2 -> Step 3 -> Step 4 -> Step 5   (sequential: each builds on the previous)
   |                                       |
   +-> Step 6 (any time)                  +-> Step 7 (planner visibility)
                                           |
                                      +-> Step 8 (struct row polymorphism)
```

Note: Step 2 introduces generics for the black box signature language. Step 5's bidirectional checker must integrate with these generics. Designing Step 2's generics with Step 5 in mind avoids rework.

## 20. Expert Review Notes

**Reviewer:** Claude (prompted as PL/compiler/SQL expert)
**Date:** April 2026

The following observations are areas where the design is technically coherent but where deeper tensions, underestimated difficulty, or alternative framings deserve attention during experimentation.

### A. Fragment Sort Gaps

**The Predicate removal and AggExpr retention are in tension.** The paper argues `Expr<Boolean>` suffices because SQL syntax already enforces predicate positions. But `AggExpr<T>` exists precisely because aggregate context matters and SQL syntax alone doesn't prevent the confusion. If aggregation context is worth a sort, predicate context might be too -- `WHERE count(*) > 5` vs. `HAVING count(*) > 5` is a common source of confusion that `Expr<Boolean>` cannot distinguish.

**Missing: `WindowExpr<T>`.** Window functions (`ROW_NUMBER() OVER (...)`, `LAG(...) OVER (...)`) are not aggregates and cannot appear in WHERE. The `sessionize` example uses them extensively. Window expressions are one of the most common sources of sort errors in SQL (using them in WHERE, GROUP BY, or nested inside aggregates). A `WindowExpr<T>` sort would catch these at composition time.

**~~`SelectItems` is under-specified.~~** *(Resolved — April 21, 2026. See §16 decision 24. `SelectItems<K, ctx>` carries a kind parameter `K ∈ {Scalar, Agg, Window}` defined as the ceiling of the contained expression sorts, with linear subtyping `SelectItems<Scalar, ctx> <: SelectItems<Agg, ctx> <: SelectItems<Window, ctx>` parallel to the expression chain. Mixed lists — e.g., a `GROUP BY` column alongside a `COUNT(*)` — have kind `Agg` by ceiling, without any per-element tag.)*

### B. Scoping Edge Cases

**Name shadowing between parameters and columns.** *(Partially addressed — April 18, 2026: parameters-first resolution with shadow warnings. See §7. The parameter always wins; the compiler warns on collisions; authors qualify with `source.column_name` to access shadowed columns. The footgun remains for Tier 2 checking with unknown schemas, but the warning makes it visible.)*

**Tier 1 + unannotated TableExpr is the worst-case error combination.** When the body says `revenue - cost` and the parameter is bare `source: TableExpr`, the checker cannot verify these columns exist without the call site. If the function is called from five models and one lacks `revenue`, the error fires inside expanded code and must be traced back -- possible but confusing. The paper should be explicit that this is the worst error experience the system produces.

**CTE-derived contexts create a forward reference problem.** In `session_rollup`, `metrics: SelectItems<Agg, sessionized>` references the CTE `sessionized` defined later in the body. The compiler must analyze the body to extract CTE schemas, then circle back to validate context bindings. This is not a single-pass analysis and could produce cyclic dependencies if a CTE's schema depends on a parameter that depends on a CTE.

### C. Bidirectional Checking: Right for MVP, Not Necessarily Forever

The argument against HM is sound for the current design. But the claim that "errors are always local" (section 9) is overstated. When a row variable `..r` is bound at one parameter and used at the return type, and the return type does not match downstream expectations, the error must reference both the binding site and the use site -- a two-point error.

Once row polymorphism, context bindings, and struct spread are all in play, the type relationships may not remain "simple enough" for bidirectional checking without heuristics. The paper should frame bidirectional checking as "right for the MVP" rather than "the right algorithm." A constraint-based approach would let row variables be solved globally, which is useful when multiple row-polymorphic parameters interact.

### D. Tier Interaction: A Tier 2 Body Calling a Tier 1 Function

The three-tier system is well-designed in isolation, but what happens when a Tier 2 function (parameters annotated, body checked in isolation) calls a Tier 1 function (unannotated, no declared types)? The Tier 2 body check cannot expand the Tier 1 call without a concrete call site. Options: (a) treat the Tier 1 return as unknown, breaking isolation; (b) refuse the call; (c) require callees to be at least the caller's tier. This interaction needs to be specified.

Separately, upgrading a Tier 1 function to Tier 2 is a potentially breaking change for callers -- adding parameter types may reject arguments that previously worked through expansion. This is the TypeScript `--strict` problem at the function level.

### E. Planner Soundness

**Property correctness is unverified.** If an author declares `joins: { dim_customers: { type: LEFT, cardinality: 1:1 } }` (structured frontmatter, shape TBD per §16 decision 22) but the join is actually 1:N, the planner will produce incorrect optimizations. There is no mechanism described for validating declared properties against the function body. The correctness of the planner depends on the correctness of hand-written property declarations -- a soundness hole.

**The MLIR analogy needs a stronger contract.** MLIR's progressive lowering has formal specifications and verified lowering passes. The paper's three levels have no formal contract beyond "the output schema must match." Schema preservation is necessary but not sufficient -- a planner rule that preserves the schema but changes row counts or NULL semantics is still wrong.

**Join elimination requires DAG-wide provenance.** Example 3's "no column from dim_products is used by any downstream consumer" requires analysis across the full model DAG, not just the immediate function call. Any model referencing this model via `smelt.ref()` is a downstream consumer.

### F. Block Syntax Ambiguity

*(Substantially addressed — April 18, 2026: PASSING keyword replaces WITH. See §10.)*

**~~WITH clause collision.~~** Resolved by using `PASSING name AS (...)` instead of `WITH name AS (...)`. The `PASSING` keyword is unambiguous — the parser distinguishes it from SQL CTEs without needing to know the function's parameter names. The parser operates independently of the type checker.

**Block composition is visually improved.** `PASSING metrics AS (metrics)` is still somewhat opaque (a parameter reference being passed through), but no longer looks like a circular CTE definition. The `PASSING` keyword signals "binding values into a parameterized context" (following SQL/XML precedent).

### G. SQL Edge Cases

**NULL semantics.** *(Explicitly deferred — April 19, 2026. See §16, decision 10. Nullability tracking is not in v1; all expressions are implicitly nullable.)*

**Implicit coercions.** *(Resolved — April 19, 2026. See §16, decision 9. `Numeric` is a type constraint, not a concrete type — `Expr<Numeric>` accepts `Expr<Integer>` via constraint satisfaction. Concrete types don't implicitly widen — `Expr<Double>` rejects `Expr<Integer>`. Built-in operators compute LUB via linear promotion chain.)*

**Dialect differences in expansion.** "Functions compile away" to plain SQL, but plain SQL differs by engine. A function body using `INTERVAL '30 minutes'` (DuckDB syntax) cannot run on Spark. Either functions are engine-agnostic or dialect-tagged.

**Window functions in bodies.** *(Resolved — April 19, 2026. See §16, decision 8. `WindowExpr<T>` added as a third expression sort with linear subtyping: `Expr<T> <: AggExpr<T> <: WindowExpr<T>`.)*

### H. Implementation Complexity

**Error tracing through expansion** requires maintaining source maps through expansion. For nested calls (A calls B calls C), even the single-level trace (A->C) requires structured expansion tracking -- a meaningful compiler infrastructure investment.

**Salsa integration** needs careful design. When a function body changes, all transitive callers must be re-checked. A `function_signature()` Salsa query separate from `function_body_check()` would ensure that body changes (without signature changes) don't invalidate call sites -- critical for LSP responsiveness.

**File discovery.** The paper says definitions can live alongside models in any `.sql` file, not just under `functions/`. This means the parser must scan every `.sql` file for `smelt.define` directives. If definitions and models coexist, the directory-derived namespace convention needs clarification for functions defined outside `functions/`.

### I. The Hardest Problem

The hardest single implementation challenge is not errors or the type system -- it is **making structural column resolution work reliably across the Tier 1 / Tier 2 boundary with the hybrid scoping model.** When a Tier 1 function with bare `source: TableExpr` references `revenue`, and the function is called from five models with different tables, the system must: expand at each call site, resolve columns against each schema, produce errors at the correct call site if a column is missing, trace errors back through expansion, and do all this incrementally for the LSP. The paper treats this as the simple base case, but it is where the most things can go wrong.

### J. Dhall's Lesson Cuts Both Ways

The paper correctly draws on Dhall's demonstration that totality + modest types can replace complex templating. But Dhall also showed the *limits* of this: users frequently hit the expressiveness ceiling and resort to workarounds. smelt's escape hatch ("drop down to a plain SQL model") is much better than Dhall's, and is worth stating explicitly as a design property.

### K. Function Versioning

Once functions are shared, changing a function's body (even without changing its signature) can break downstream models silently. The planner story makes this worse: changing a body might invalidate planner annotations, leading to incorrect optimizations. Whether function signatures should include a version or hash, or whether the planner should re-verify annotations when bodies change, is worth considering.

### L. Unified Model Implications

**The refs-as-defaults equivalence is validated by testing but not yet by user experience.** The testing framework's ability to provide mock tables with subset schemas proves the technical equivalence, but the user-facing syntax and mental model need careful design. Making every model a "parameterizable function" risks confusing users who think in terms of a simple DAG. The default experience should remain `smelt.ref('x')` with no visible parameterization; the unified view should be an advanced capability, not the primary teaching model.

**Ephemeral models and transparent functions have different lifecycle expectations.** An ephemeral model is scheduled in the DAG (it has a position in execution order) even though it's inlined. A transparent function is not scheduled -- it's expanded at each call site. Under the unified view, this distinction becomes "materialized ephemeral" (scheduled, inlined) vs "pure inline" (not scheduled, expanded). Whether the scheduler needs to be aware of pure inline functions (e.g., for cycle detection across function calls) needs specification.

**Black box soundness depends on signature correctness.** For SQL built-ins, the signature registry can be validated against engine documentation or introspection. For `smelt.extern` UDFs, the declared signature is unverified -- if the user declares the wrong return type, downstream type checking is unsound. This is the same problem as planner annotation correctness (section E) but more pervasive, since every UDF call depends on it. Runtime schema validation (check actual output against declared type on first execution) could provide a safety net.

**The signature language for generics is on the critical path.** Moving built-in typing from Step 8 to Step 2 means the generics design must be settled early. Getting generics wrong (too simple, too complex, or incompatible with bidirectional checking) has cascading effects on every subsequent step. The design should study TypeScript's approach to generic inference in function calls, which solves a similar problem (infer type parameters from argument types at call sites).

## 21. Pre-Implementation Design Checklist

*Added April 19, 2026. Items that need discussion/decision before creating an implementation plan. Check off items as they are resolved and add decisions to §16.*

### Must resolve (blocks implementation)

- [x] **`smelt.define` grammar.** Resolved April 19, 2026 — see §16 decision 11. Top-level statement with the shape `smelt.define <name>(<params>) [-> <type>] AS (<body>) [;]`. Parenthesized body is required (single-token termination). Files are a sequence of top-level items; frontmatter applies only to the model `SELECT`. `smelt.define` is the sync token for error recovery.

- [x] **Expansion mechanics.** Resolved April 19, 2026 — see §16 decision 12. AST-level rewriting on the CST with structured provenance tags per node. Lazy: calls stay symbolic through Level 1, expand at Level 2. Tier 1 type checking uses a type-context binding rather than materializing an expanded CST. Textual substitution rejected: it would rebuild source maps, reparse synthesized fragments, and lose CST position tracking.

- [x] **Tier 1 error tracing: Step 1 MVP scope.** Resolved April 19, 2026 — see §16 decision 16. Step 1 ships single-level traces (call site → innermost error with the parameter binding that triggered it). The frame stack from decision 12 is populated from day one; the multi-level renderer is added in Step 2 as mechanical diagnostic polish.

### Must resolve (blocks Step 2 — built-in typing)

- [x] **`Ordered` constraint specification.** Resolved April 19, 2026 — see §16 decision 13. Members: `Numeric ∪ {Text, Date, Time, Timestamp, Boolean, Interval, Binary}`. Composites (`Struct`, `Array`, `Map`) excluded in v1 due to cross-backend divergence. Collation for `Text` deferred as a separate typed property.

- [x] **Generics syntax and inference.** Resolved April 19, 2026 — see §16 decision 14. Angle-bracket generics on signatures only (`MIN<T: Ordered>(T) → T`); `smelt.define` stays monomorphic in v1. Inference collects positions for each type parameter; bound by LUB where the constraint has a promotion chain (Numeric), otherwise by unification. Expected return type in checking mode participates as an additional position.

- [x] **Variadics.** Resolved April 19, 2026 — see §16 decision 15. Trailing `...` on the final argument position; minimum arity = number of preceding required positions. Variadics in argument positions only, built-ins and `smelt.extern` only, positional-only. Variadic expansion feeds decision 14's inference rule unchanged.

### Must resolve (blocks Step 3 — TableExpr functions)

- [x] **Tier interaction: Tier 2 calling Tier 1.** Resolved April 19, 2026 — see §16 decision 17. The Tier 1 callee is expanded inline during the Tier 2 body check, using the Tier 2 function's declared parameter types as the concrete argument types at that expansion site. Errors surface against the Tier 2 body with the frame-stack trace from decisions 12/16; Tier 2's signature is unaffected.

### Should resolve (reduces risk)

- [x] **`smelt.extern` full syntax.** Resolved April 20, 2026 — see §16 decision 21. Grammar is `smelt.extern <name>(<params>) -> <return-type> [;]` with optional per-declaration frontmatter for per-backend emission rules. File placement follows decision 11 (any `.sql` file, coexists with models and defines). Call surface is bare-name, not via `smelt.fn.*`. Backend namespace (`duckdb.foo`) is sugar for frontmatter-based emission. Type-checker treatment is identical to SQL built-ins.

- [x] **Unify property mechanism on frontmatter; remove inline annotations.** Resolved April 20, 2026 — see §16 decision 22. All `@annotation` syntax is removed; properties live in a frontmatter block that attaches to the immediately following declaration (model, `smelt.define`, or `smelt.extern`). Supersedes decision 11's scoping of frontmatter to the model `SELECT` only. Single mechanism, structured data works naturally, annotations can return later as pure sugar if ergonomic evidence demands it.

- [x] **`smelt.as_struct()` semantics.** Resolved April 19, 2026 — see §16 decision 19. Deferred to post-v1. §6 Strategy 3 drops out of the v1 surface; Strategies 1 (explicit CTE with aliases) and 2 (typed `TableExpr<{...}>`) remain as the v1 options for multi-join functions. Revisit alongside Step 8 (struct row polymorphism).

- [x] **PASSING keyword parsing details.** Resolved April 19, 2026 — see §16 decision 18. Context-sensitive: `PASSING` is reserved only immediately after the `)` closing a `smelt.fn.*` call or `smelt.define`-declared call; everywhere else it is a regular identifier. The trigger is purely syntactic (namespace-prefixed call path), so the parser stays independent of the type checker.

- [x] **Default value expansion for fragment sorts.** Resolved April 20, 2026 — see §16 decision 20. Defaults are explicit (no implicit empty for list sorts), type-checked at definition time, bound at call resolution, cloned into placeholder positions at Level 2 with `Synthesized` provenance per decision 12. List splices elide adjacent commas syntactically when a splice contributes zero elements; non-empty context-bound defaults share §6's call-site-deferred column-name validation.

- [x] **Dialect differences in function bodies.** Resolved April 19, 2026 — see §16 decision 23. Engine-agnostic canonical SQL bodies; engine-specific features reached only through backend namespace (`duckdb.*` etc.); `backends:` frontmatter is inferred as the intersection of call-site backends and may be narrowed but not widened; no per-function dialect tag; translation is a single pass at final expansion. "Canonical SQL" is defined pragmatically as whatever the translation layer can emit faithfully across supported backends.

- [x] **SelectItems kind parameter.** Resolved April 21, 2026 — see §16 decision 24. `SelectItems<K, ctx>` carries a kind parameter (Scalar / Agg / Window = ceiling of contained expression sorts) with linear subtyping parallel to the expression chain, preserving sort information across list-valued parameters and splice points. Also tightens §2's aggregate-context definition to cover the implicit single-group case (a SELECT with no `GROUP BY` whose items are all aggregated).

### Can defer (resolve during implementation)

- [ ] **CTE forward reference / cycle detection.** `metrics: SelectItems<Agg, sessionized>` references a CTE defined later in the body. The compiler must handle forward references without creating cycles in its own analysis. Solvable but needs care.

- [ ] **Function file discovery.** Definitions can live alongside models in any `.sql` file. The parser must scan every `.sql` file for `smelt.define`. How does this interact with the directory-derived namespace convention for functions defined outside `functions/`?

- [ ] **`AggExpr<T>` — keep or collapse?** §18 flags this. Same argument as the Predicate removal: aggregation context is enforced by SQL syntax. Counter-argument: the linear subtyping chain (§16 decision 8) gives AggExpr a clear role. Probably keep, but can decide during implementation.

- [ ] **Upgrade path: Tier 1 → Tier 2 breaking changes.** Adding parameter types to a Tier 1 function may reject arguments that previously worked. This is the TypeScript `--strict` problem at the function level. Needs a migration story but not blocking for v1.
