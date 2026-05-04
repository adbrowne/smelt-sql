# smelt Functions

smelt functions let you define reusable SQL fragments that are checked by the compiler. They remove copy-paste duplication, add type safety at call sites, and expose structured metadata to the planner for optimizations like filter pushdown and join elimination.

## Defining a function

Functions live in a `functions/` directory alongside your `models/` directory. Each `.sql` file can contain one or more `smelt.define` declarations.

```
my_project/
  models/
    orders.sql
  functions/
    safe_divide.sql
    sessionize.sql
  smelt.yml
```

### Basic syntax

```sql
smelt.define function_name(param1, param2) AS (
  -- SQL body using parameter names
  param1 / NULLIF(param2, 0)
)
```

### Typed parameters (recommended)

Annotating parameters with types lets smelt check that callers pass compatible values and check the body in isolation:

```sql
smelt.define safe_divide(
  numerator: Expr<Numeric>,
  denominator: Expr<Numeric>
) -> Expr<Double> AS (
  CASE WHEN denominator = 0 OR denominator IS NULL
    THEN NULL
    ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
  END
)
```

### Default parameter values

Parameters can have default values, making them optional at call sites:

```sql
smelt.define sessionize(
  source: TableExpr,
  user_col: Expr<Text>,
  ts_col: Expr<Timestamp>,
  gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
  SELECT
    source.*,
    SUM(CASE WHEN ts_col - LAG(ts_col) OVER (PARTITION BY user_col ORDER BY ts_col) > gap
             THEN 1 ELSE 0 END)
      OVER (PARTITION BY user_col ORDER BY ts_col) AS session_id
  FROM source
)
```

## Calling a function

Use `smelt.functions.<name>()` to call a user-defined function:

```sql
-- In a model file
SELECT
  smelt.functions.safe_divide(revenue, cost) AS margin
FROM smelt.orders
```

A function's call path is derived from the workspace-relative directory of the file it is declared in, joined with the declared name. The filename stem itself is **not** part of the call path. The mapping is enforced — calling a function under the wrong path is an `UnknownSmeltFn` diagnostic.

| Filesystem location | Declared name | Call path |
|---|---|---|
| `functions/safe_divide.sql` | `safe_divide` | `smelt.functions.safe_divide(...)` |
| `functions/status.sql` | `is_shipped` | `smelt.functions.is_shipped(...)` |
| `functions/patterns/x.sql` | `session_rollup` | `smelt.functions.patterns.session_rollup(...)` |
| `utils/math.sql` | `safe_divide` | `smelt.utils.safe_divide(...)` |

Renaming a function or moving its file changes the call path, the same way moving a model does.

### Verifying function calls

Before doing a full `smelt build`, confirm that a function call expands correctly using `--show-plan`:

```bash
smelt build --show-plan models/<model>.sql
```

The `ExpandedCall` node in the plan output shows the inlined function body with argument substitution already applied. This is faster than a full build and catches wrong-path errors (such as `UnknownSmeltFn`) without touching the database.

Note that `--show-plan` requires a positional model file path — there is no project-wide show-plan mode. See [`smelt build`](../reference/cli.md#smelt-build) in the CLI reference for details.

### Declared return types and model schemas

For typed functions (those with a `-> ReturnType` annotation), smelt uses the declared return type as the column type in downstream models. `smelt table <model>` reflects this — a column fed by a `-> Expr<Double>` call shows as `DOUBLE`. Downstream aggregates also use the declared type: `SUM` over a `-> Expr<Double>` call infers as `DOUBLE`, not `BIGINT`.

### Calling in boolean positions

A function whose declared return type is `Expr<Boolean>` can be used in any boolean position the SQL grammar accepts: `WHERE`, `HAVING`, `JOIN ON`, `QUALIFY`, `CASE WHEN`, and as a `SELECT`-list expression.

```sql
-- functions/status.sql
smelt.define is_shipped(status: Expr<Text>) -> Expr<Boolean> AS (
  status = 'shipped' OR status = 'delivered'
)
```

```sql
-- models/shipped_orders.sql
SELECT *
FROM smelt.orders
WHERE smelt.functions.is_shipped(status)
```

### Named arguments

Pass arguments by name to improve readability or skip over defaulted parameters:

```sql
SELECT *
FROM smelt.functions.sessionize(
  smelt.events,
  user_col => user_id,
  ts_col   => event_time,
  gap      => INTERVAL '1 hour'
)
```

## Three tiers of annotation

smelt uses gradual typing — you choose how much annotation to add.

### Tier 1 — unannotated (quick and personal)

```sql
smelt.define add_one(x) AS (x + 1)
```

- No type annotations on parameters or return type.
- The body is expanded at each call site with the caller's concrete types substituted in.
- Type errors only surface at call sites where the concrete types cause a problem.
- Good for quick personal utilities or exploratory work.

### Tier 2 — parameters annotated (production code)

```sql
smelt.define add_margin(
  revenue: Expr<Numeric>,
  cost: Expr<Numeric>
) AS (revenue - cost)
```

- Every parameter is annotated.
- The body is type-checked once at definition time against the declared parameter types.
- Errors are reported against the function body, not each call site.
- Callers whose argument types don't satisfy the constraints get an `ArgTypeMismatch` diagnostic immediately.

### Tier 3 — fully annotated (library quality)

```sql
smelt.define safe_divide(
  numerator: Expr<Numeric>,
  denominator: Expr<Numeric>
) -> Expr<Double> AS (
  CASE WHEN denominator = 0 OR denominator IS NULL
    THEN NULL
    ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
  END
)
```

- Both parameters and return type are annotated.
- The return type is verified against the body — if the body evaluates to a different type, you get a `ReturnTypeMismatch` diagnostic at the function declaration.
- LSP hover on a call site shows the declared return type directly.

See [`docs/smelt-functions-upgrade-story.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/smelt-functions-upgrade-story.md) for how to migrate from Tier 1 to Tier 2 without breaking existing callers.

## Type constraints

The type language for parameter and return annotations:

| Annotation | Meaning |
|---|---|
| `Expr<Integer>` | Any integer expression |
| `Expr<Numeric>` | Any numeric expression (Integer, BigInt, Float, Double, Decimal) |
| `Expr<Double>` | A double-precision float expression |
| `Expr<Text>` | A text/varchar expression |
| `Expr<Boolean>` | A boolean expression |
| `Expr<Timestamp>` | A timestamp expression |
| `Expr<Date>` | A date expression |
| `Expr<Interval>` | An interval expression |
| `Expr<Any>` | Any scalar expression type |
| `TableExpr` | A table-valued argument (bare row polymorphism) |
| `TableExpr<{col: Type, ..r}>` | A table with at least the listed columns |
| `AggExpr<T>` | An aggregate expression |
| `WindowExpr<T>` | A window (analytic) expression |
| `SelectItems<K>` | A SELECT-list fragment with kind ceiling `K` (`Scalar`, `Agg`, `Window`) |
| `SelectItems<K, ctx>` | As above, but columns must belong to context `ctx` |

## Fragment parameters — TableExpr and SelectItems

Fragment sorts are the key to composable pipelines. They let you pass table-valued arguments and SELECT-list fragments to functions.

### TableExpr — table-valued parameters

`TableExpr` parameters accept a table reference (`smelt.<name>`, `smelt.sources.<name>`, a CTE, or a subquery):

```sql
smelt.define add_margin(
  source: TableExpr<{revenue: Numeric, cost: Numeric}>
) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin
  FROM source
)
```

Call it with any table that has at least those columns:

```sql
SELECT * FROM smelt.functions.add_margin(smelt.orders)
```

The row-requirement annotation `TableExpr<{revenue: Numeric, cost: Numeric}>` is checked at each call site — the compiler reports a `RowRequirementMissing` diagnostic if the supplied table is missing a required column.

### SelectItems — SELECT-list fragments

`SelectItems<Agg>` parameters accept aggregate expressions, passed inline or via a `PASSING` clause:

```sql
smelt.define session_rollup(
  source: TableExpr,
  user_col: Expr<Text>,
  ts_col: Expr<Timestamp>,
  gap: Expr<Interval> = INTERVAL '30 minutes',
  metrics: SelectItems<Agg, sessionized> = ()
) -> TableExpr AS (
  WITH sessionized AS (
    SELECT * FROM smelt.functions.sessionize(source, user_col, ts_col, gap)
  )
  SELECT
    user_col, session_id,
    MIN(ts_col) AS session_start, MAX(ts_col) AS session_end,
    COUNT(*) AS event_count,
    metrics
  FROM sessionized
  GROUP BY user_col, session_id
)
```

The `SelectItems<Agg, sessionized>` annotation means:
- The fragment must contain only aggregate expressions (`Agg` ceiling).
- Column references inside the fragment must be columns of the `sessionized` CTE (the *context*).

## PASSING clauses — block syntax for fragment parameters

Instead of passing `SelectItems` arguments inline, use a trailing `PASSING` clause for ergonomic multi-line fragments:

```sql
-- Inline style
SELECT * FROM smelt.functions.session_rollup(
  smelt.sources.events,
  user_col => user_id,
  ts_col   => event_time,
  metrics  => (COUNT(*) AS event_count, SUM(amount) AS total_amount)
)

-- Block style with PASSING
SELECT *
FROM smelt.functions.session_rollup(
  smelt.sources.events,
  user_col => user_id,
  ts_col   => event_time
) PASSING metrics AS (
  COUNT(*) AS event_count,
  SUM(amount) AS total_amount
)
```

Multiple PASSING clauses are allowed (one per fragment parameter):

```sql
FROM smelt.functions.session_rollup(source, user_id, ts)
PASSING metrics AS (COUNT(*), SUM(revenue))
PASSING filters AS (amount > 0)
```

PASSING clauses are type-checked identically to inline arguments — the compiler verifies column references against the declared context.

## External functions — smelt.extern

Declare a backend-native function that has no smelt body. This gives the compiler a typed signature for call-site checking without requiring an implementation:

```sql
smelt.extern regex_match(
  text: Expr<Text>,
  pattern: Expr<Text>
) -> Expr<Boolean>
```

The function is then callable as `smelt.functions.regex_match(col, 'pattern')` with full type checking at call sites. Unlike `smelt.define`, there is no `AS (...)` body.

!!! note
    `smelt.extern` only accepts scalar (`Expr<T>`) and table (`TableExpr`) parameter types. Fragment-sort parameters (`SelectItems`, `AggExpr`, `WindowExpr`) are not supported on extern declarations.

## Struct packing — smelt.as_struct()

`smelt.as_struct(alias [EXCEPT col1, col2, ...])` converts a table reference into a struct value, optionally excluding columns. This is useful for passing all columns from a join source into a single struct field:

```sql
smelt.define enrich_order_with_as_struct(
  orders: TableExpr<{order_id: BigInt, customer_id: Text, total: Numeric}>,
  customers: TableExpr<{customer_id: Text, customer_name: Text, customer_tier: Text}>
) -> TableExpr AS (
  SELECT
    smelt.as_struct(o EXCEPT customer_id) AS order_data,
    smelt.as_struct(c EXCEPT customer_id) AS customer_data
  FROM orders AS o
  JOIN customers AS c ON o.customer_id = c.customer_id
)
```

The emitted SQL uses backend-specific struct literal syntax:
- **DuckDB**: `{'order_id': o.order_id, 'total': o.total}`
- **Spark**: `struct(o.order_id AS order_id, o.total AS total)`
- **Postgres**: row constructor syntax

!!! warning
    `smelt.as_struct` requires a backend that supports struct literals. Declare the function's `backends:` frontmatter to restrict it to compatible backends (see [Backends frontmatter](#backends-frontmatter) below), or ensure your `smelt.yml` only targets backends that support struct literals.

## Frontmatter for functions

Functions can have per-declaration YAML frontmatter. Place it immediately before the `smelt.define` or `smelt.extern` declaration it applies to.

### backends:

Restrict a function to one or more backends:

```sql
---
backends: [duckdb]
---
smelt.define safe_divide(n: Expr<Numeric>, d: Expr<Numeric>) -> Expr<Double> AS (
  CASE WHEN d = 0 OR d IS NULL THEN NULL
       ELSE CAST(n AS DOUBLE) / CAST(d AS DOUBLE)
  END
)
```

When `backends:` is declared, the compiler emits a `BackendsMismatch` diagnostic if the function is called from a model targeting a different backend.

### deterministic: true / false

Marks whether the function returns the same result for the same inputs (default: unknown). The planner uses this to decide whether filters can be pushed across a function call boundary.

```sql
---
deterministic: true
---
smelt.define event_hour(ts: Expr<Timestamp>) -> Expr<Integer> AS (
  EXTRACT(HOUR FROM ts)
)
```

### provenance: and joins:

Declares column-level lineage and join relationships for the planner. Requires `unstable_schema: true` in `smelt.yml`.

```sql
---
deterministic: true
provenance: { margin: [source.revenue, source.cost] }
---
smelt.define add_margin_with_provenance(
  source: TableExpr<{revenue: Numeric, cost: Numeric}>
) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
```

## Diagnostic reference

| Code | Meaning | Fix |
|---|---|---|
| `ArgTypeMismatch` | Argument type doesn't satisfy the parameter constraint | CAST the argument or widen the annotation |
| `MissingArgument` | A required parameter was not supplied | Provide the argument or add a default |
| `UnknownSmeltFn` | `smelt.functions.name` not found in any function file | Check the name and ensure the file is in `functions/` |
| `FunctionBodyTypeMismatch` | A subexpression in the body has an unexpected type | Fix the body expression |
| `ReturnTypeMismatch` | Body evaluates to a type incompatible with the `-> Expr<T>` return annotation | Adjust the body or the declared return type |
| `RowRequirementMissing` | A `TableExpr` argument is missing a required column | Ensure the table has the column, or relax the row-requirement annotation |
| `ParameterShadowsColumn` | A parameter name matches a bare column in the `TableExpr` schema | Rename the parameter or qualify the column reference |
| `DuplicateFunctionDefinition` | Two `smelt.define` declarations share a name | Rename one of them |
| `BackendsMismatch` | Function's `backends:` is incompatible with the call site's target | Ensure the function supports the target backend |
| `ExternFragmentParamUnsupported` | `smelt.extern` declares a `SelectItems` / fragment-sort parameter | Remove the fragment parameter; extern functions are scalar/table-only |
| `FragmentColumnMissing` | Column referenced in a `PASSING` body isn't in the declared context | Correct the column name or the context annotation |
| `FunctionCallCycle` | A function directly or indirectly calls itself | Restructure to eliminate the cycle |

## Minimal end-to-end example

A complete self-contained project showing the full file → define → call cycle:

```
my-project/
  smelt.yml
  seeds/
    raw_orders.csv        # order_id, customer_id, status, amount
  functions/
    revenue.sql
    status.sql
  models/
    stg_orders.sql
    mart_revenue.sql
```

```sql
-- functions/revenue.sql
smelt.define safe_revenue(amount: Expr<Double>) -> Expr<Double> AS (
  COALESCE(amount, 0.0)
)
```

```sql
-- functions/status.sql
smelt.define is_shipped(status: Expr<Text>) -> Expr<Boolean> AS (
  status = 'shipped'
)
```

```sql
-- models/stg_orders.sql
---
name: stg_orders
materialization: table
---
SELECT
  order_id,
  customer_id,
  smelt.functions.safe_revenue(CAST(amount AS DOUBLE)) AS amount,
  status
FROM smelt.raw_orders
```

```sql
-- models/mart_revenue.sql
---
name: mart_revenue
materialization: table
---
SELECT
  customer_id,
  CAST(COUNT(*) AS INTEGER) AS order_count,
  SUM(amount) AS total_revenue
FROM smelt.stg_orders
WHERE smelt.functions.is_shipped(status)
GROUP BY customer_id
```

Key rules demonstrated:
- `functions/` is auto-discovered — no `smelt.yml` change needed.
- Call path = `smelt.functions.<declared_name>` — the **filename stem is not included**.
- `Expr<Boolean>` works directly in `WHERE` with no extra wrapping.
- Arguments are positional in v1 (`param => value` named syntax is not yet wired end-to-end).
