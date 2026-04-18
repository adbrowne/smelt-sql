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
| `AggExpr<T>` | Expression containing aggregation | SELECT (with GROUP BY), HAVING |
| `TableExpr` | Something with a schema | FROM, JOIN, WITH |
| `SelectItems` | List of (expression, alias) pairs | SELECT clause |
| `OrderSpec` | Expression + direction | ORDER BY |

These sorts ensure structural well-formedness: you cannot splice a `TableExpr` into a WHERE clause, or an `Expr<Boolean>` into a FROM clause. The compiler checks sort-correctness at each composition point.

A bare column reference like `user_id` is a trivial `Expr<T>` -- there is no separate `Column<T>` sort. This keeps the sort system minimal. If a function parameter is spliced into `PARTITION BY` or `GROUP BY`, the author documents the expectation via naming and comments, not via the type system. (A `Column<T>` subtype could be reintroduced later if experience shows the distinction is valuable, but the simpler model is the right starting point.)

A `Predicate` sort was considered and rejected. Use `Expr<Boolean>` instead -- the positional constraint (WHERE, ON, HAVING) is already enforced by SQL syntax, and one fewer concept to learn is worth more than one more sort.

The initial implementation targets a subset: `Expr<T>` and `TableExpr`. The remaining sorts (`AggExpr<T>`, `SelectItems`, `OrderSpec`) are added once basic function composition is validated. Each sort is independently addable without breaking existing functions.

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

The planner treats black box functions as optimization barriers -- it cannot rewrite what's inside. However, black box functions can still carry declared properties:

- `@deterministic` -- the planner knows the function produces the same output for the same input
- `@idempotent` -- safe to retry
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

Explicit context annotations serve two purposes:
1. **Documentation** -- making the contract visible in the signature
2. **Validation** -- the compiler checks the annotation matches the inferred context

Explicit annotations are **required** when a parameter is used in multiple scopes (e.g., spliced into two different CTEs with different schemas). In this case the compiler cannot infer a single context, and the annotation disambiguates.

### The Key Insight: Asymmetric Access Control

Consider `session_rollup`:

```sql
smelt.define session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean> = TRUE
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

**Strategy 3: `smelt.as_struct()` for compile-time namespacing.** When multiple tables need to be accessible without column name collisions, wrap each table's columns into a struct:

```sql
smelt.as_struct(source EXCEPT customer_id, product_id)
-- produces: STRUCT(col1, col2, ...) excluding the join keys
```

`smelt.as_struct()` is a **compile-time construct** with zero runtime cost -- the compiler knows the concrete struct fields at expansion time and generates explicit field references. This provides SQL-safe namespacing (`source_struct.revenue`, `customer_struct.segment`) without runtime struct creation overhead.

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
- **Implicit subtyping coercions.** The checker does not silently insert casts. If a parameter expects `Expr<Double>` and the caller passes `Expr<Integer>`, this is a type error. The user writes `CAST(x AS DOUBLE)`. (Exception: engine aliases like `Text`/`Varchar` are treated as the same type.)

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

### What Ships When

**MVP (Steps 1-5):** Pure expansion for transparent functions, signature checking for black box functions. No planner rules at any level. Bare keyword annotations (`@deterministic`, `@idempotent`, `@append_only`) are parsed and stored but not acted on.

**Post-MVP (Step 7):** Level 1 planner rules. Structured annotations (`@joins(...)`, `@provenance(...)`) for the functions that benefit from optimization. The transparency rule guides which functions the planner can reason through. Levels 2 and 3 build on existing planner infrastructure.

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

**`@backends` as a function property.** Backend compatibility uses the same `@annotation` system as `@deterministic` and `@idempotent`:

```sql
COALESCE(a, b)                    -- @deterministic @backends(all)
MEDIAN(col)                       -- @deterministic @backends(duckdb, postgres)
duckdb.read_parquet('*.parquet')  -- @backends(duckdb)
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

Properties flow through all three levels: `@deterministic` tells Level 3 replaying a failed batch is safe; `@append_only` on `source` tells Level 2 incremental processing is valid.

### Example 3: Join Elimination via Function-Aware Planning

*Note: This example illustrates a future capability (Step 7 in the roadmap). It requires planner integration with structured annotations (`@provenance`, `@joins`), which are post-MVP.*

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

The compiler infers a parameter's context from where it is spliced in the function body. Explicit context annotations are optional (for documentation and validation), required only when a parameter is used in multiple scopes.

**Rationale:** Requiring explicit context annotations on every fragment parameter would be a significant annotation burden with diminishing returns — most parameters are used in exactly one place. Inference from the splice point gives the compiler the same information automatically. Explicit annotations remain available for documentation and for the case where a parameter appears in multiple scopes (where inference is ambiguous).

### 6. Multiple defines per file; smelt.metric() out of scope (§3)

A `.sql` file may contain multiple `smelt.define` definitions. `smelt.metric()` is independent from functions and not addressed by this design.

**Rationale:** Multiple defines per file is consistent with how models already work — a file is a compilation unit, not a one-definition container. `smelt.metric()` doesn't work today and has different design constraints (it's a semantic layer concept, not a composition mechanism); conflating the two would complicate both designs.

### 7. Bare columns from TableExpr allowed when unambiguous (§7)

Bare column references from `TableExpr` parameters resolve through standard SQL column resolution when no parameter name matches. Qualification is required only when a bare column name overlaps with a parameter name.

**Rationale:** Consistent with the no-overlap rule (§6) and parameters-first scoping. SQL developers expect bare column names to work — requiring qualification everywhere would be hostile. The parameters-first rule means parameters shadow columns, and the shadow warning catches accidental collisions. Qualification is the author's escape hatch, not a default burden.

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
- **Structured annotations** (`@joins(...)`, `@provenance(...)`) -- deferred until the planner needs them.
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

**Build:** Canonical signature registry for SQL built-ins (one registry, not per-dialect). The ~80% of built-ins with simple signatures (`Expr<T> -> Expr<T>`, `Expr<T> -> AggExpr<T>`). `smelt.extern` for user-declared black box functions. Generics/type parameters for the ~20% that need them (`COALESCE<T>`, `ARRAY_AGG<T>`). `@backends` annotations for portability tracking. Backend namespace (`duckdb.*`, `postgres.*`) for engine-specific functions and native-precision opt-in. CAST enforcement for canonical return types.

**What we learn:** Can the fragment sort system extend to cover SQL built-ins? Does the signature language need variadics immediately, or can fixed-arity overloads suffice for MVP? Is the `smelt.extern` declaration natural for UDFs? What is the effort/value ratio of each signature language extension (generics, variadics, type-as-arguments)? Does the canonical-type-with-CAST approach produce correct schemas across backends? Is the backend namespace natural for opting into engine-specific behavior?

**How it ladders:** This is mandatory infrastructure -- every subsequent step benefits from built-in type information flowing through the checker. Generics here also inform the Tier 2 bidirectional checker (Step 5). Black box functions are simpler than transparent functions (no expansion, no body analysis), so this is a good early test of the signature language before applying it to the harder transparent case. The `@backends` property establishes the portability model that the planner needs for multi-backend execution.

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

**`SelectItems` is under-specified.** The `Agg` kind parameter is never formally defined. Can there be `SelectItems<Scalar, ctx>`? What about mixed select lists (some aggregate, some GROUP BY columns)? Real SELECT clauses regularly mix both.

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

**Annotation correctness is unverified.** If an author declares `@joins(dim_customers LEFT 1:1)` but the join is actually 1:N, the planner will produce incorrect optimizations. There is no mechanism described for validating annotations against the function body. The correctness of the planner depends on the correctness of hand-written annotations -- a soundness hole.

**The MLIR analogy needs a stronger contract.** MLIR's progressive lowering has formal specifications and verified lowering passes. The paper's three levels have no formal contract beyond "the output schema must match." Schema preservation is necessary but not sufficient -- a planner rule that preserves the schema but changes row counts or NULL semantics is still wrong.

**Join elimination requires DAG-wide provenance.** Example 3's "no column from dim_products is used by any downstream consumer" requires analysis across the full model DAG, not just the immediate function call. Any model referencing this model via `smelt.ref()` is a downstream consumer.

### F. Block Syntax Ambiguity

*(Substantially addressed — April 18, 2026: PASSING keyword replaces WITH. See §10.)*

**~~WITH clause collision.~~** Resolved by using `PASSING name AS (...)` instead of `WITH name AS (...)`. The `PASSING` keyword is unambiguous — the parser distinguishes it from SQL CTEs without needing to know the function's parameter names. The parser operates independently of the type checker.

**Block composition is visually improved.** `PASSING metrics AS (metrics)` is still somewhat opaque (a parameter reference being passed through), but no longer looks like a circular CTE definition. The `PASSING` keyword signals "binding values into a parameterized context" (following SQL/XML precedent).

### G. SQL Edge Cases

**NULL semantics.** The fragment sorts do not mention nullability. When a parameter is `Expr<Numeric>`, can the caller pass a nullable expression? What is the nullability of the return type? Either nullable/non-nullable should be expressible (`Expr<Numeric NOT NULL>`) or the paper should state nullability tracking is deferred.

**Implicit coercions.** The paper says the checker does not insert casts, and `Text`/`Varchar` are treated as the same type. But what about `Integer`/`Bigint`? `Numeric`/`Decimal`? If `Expr<Numeric>` does not accept `Expr<Integer>`, users need casts everywhere, which is hostile. The numeric tower subtyping rules need specification.

**Dialect differences in expansion.** "Functions compile away" to plain SQL, but plain SQL differs by engine. A function body using `INTERVAL '30 minutes'` (DuckDB syntax) cannot run on Spark. Either functions are engine-agnostic or dialect-tagged.

**Window functions in bodies.** The `sessionize` example uses `LAG(...) OVER (...)` and `SUM(...) OVER (...)`. If a function body contains window functions, the sort system should ensure the result is used where window functions are valid. Neither `Expr<T>` nor `AggExpr<T>` captures "contains a window function."

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
