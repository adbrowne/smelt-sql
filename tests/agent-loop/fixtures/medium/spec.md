# Medium fixture: typed orders pipeline with smelt functions

You are building a smelt data project that produces the same three reports as the
small fixture, but the implementation **must** use typed `smelt.define` functions
for reusable business logic. Plain SQL with no function definitions will not pass
the acceptance check.

## Inputs

Two seed CSVs are already copied into `seeds/`:

- `seeds/raw_customers.csv` — 5 rows: `customer_id`, `name`, `country`
- `seeds/raw_orders.csv` — 10 rows: `order_id`, `customer_id`, `order_date`, `status`, `amount`

`status` is one of `shipped` or `cancelled`. `amount` is in dollars (decimal).

## Required functions

Create a `functions/` directory under the project root. Define **at least two**
typed `smelt.define` functions that your models call. The functions must carry
type annotations on their parameters and a `-> ReturnType` arrow. Suggested
starting point — you may rename, restructure, or add more:

```sql
-- functions/revenue.sql
smelt.define safe_revenue(amount: Expr<Double>) -> Expr<Double> AS (
    COALESCE(amount, 0.0)
)

-- functions/status.sql
smelt.define is_shipped(status: Expr<Text>) -> Expr<Boolean> AS (
    status = 'shipped'
)
```

Call them in your models as `smelt.functions.<name>(...)`. The filename stem is
**not** a path component — the path contains only the directory segments, not the
stem. For example, `smelt.functions.safe_revenue(o.amount)` for a function
`safe_revenue` declared in `functions/revenue.sql` (directory `functions/`,
stem `revenue` is excluded from the call path).

Rules for v1 functions (things that work today):
- Parameters must be `Expr<T>` sorts (scalar expressions only — no `TableExpr` or `PASSING` in this fixture).
- Arguments are **positional**; named `param => value` syntax is not yet wired end-to-end.
- The `functions/` directory is auto-discovered; no config change to `smelt.yml` is needed.
- A single `.sql` file may contain multiple `smelt.define` declarations.

## Required outputs

The final DuckDB database must contain these three tables:

### `stg_orders`
One row per order. Columns:

- `order_id` (INTEGER)
- `customer_id` (INTEGER)
- `customer_name` (VARCHAR) — joined from `raw_customers`
- `country` (VARCHAR) — joined from `raw_customers`
- `order_date` (DATE)
- `status` (VARCHAR)
- `amount` (DECIMAL or DOUBLE)

All 10 orders appear here (including cancelled).

### `int_orders_by_day`
Daily summary of **shipped orders only**. One row per `order_date` that has at
least one shipped order. Columns:

- `order_date` (DATE)
- `order_count` (INTEGER)
- `total_revenue` (DECIMAL or DOUBLE)

### `mart_top_customers`
One row per customer — all 5 customers, even those with no shipped orders. Columns:

- `customer_id` (INTEGER)
- `customer_name` (VARCHAR)
- `total_revenue` (DECIMAL or DOUBLE) — shipped-order revenue only; 0 for customers with none

## How to know you are done

The harness runs `validate.py` automatically after you finish — you do not need to invoke it manually. To self-check, query the output tables directly:

```bash
duckdb my-project.duckdb -c "SELECT COUNT(*) FROM stg_orders"
duckdb my-project.duckdb -c "DESCRIBE int_orders_by_day"
```

The validator also checks that a `functions/` directory with at least one
`smelt.define` declaration exists and is called from your models.

## Hints

- The skill at `<run_dir>/skill.md` covers the project skeleton, model format, and CLI flags. Read it first.
- `smelt docs show concepts/functions` explains the `smelt.define` syntax and call paths.
- `smelt docs show getting-started/quickstart` gives the minimum project skeleton.
- Build the seeds → one staging model → `smelt build` first, then layer in the functions.
- After defining a function, run `smelt build --show-plan models/stg_orders.sql` to confirm the
  call expands correctly before doing a full build.
- `smelt.functions.<name>` — the path uses only the directory segments under
  `functions/`, **not** the filename stem. A function `safe_revenue` in
  `functions/revenue.sql` is called as `smelt.functions.safe_revenue(...)`, not
  `smelt.functions.revenue.safe_revenue(...)`. Including the stem is an error.

## Optional: meta-list lift (Phase A surface)

Once the required outputs above are passing, try this extension: `stg_orders`
joins `raw_customers` and surfaces two VARCHAR identity columns —
`customer_name` and `country` — that you might repeat across intermediate CTEs.
Lift that repeated VARCHAR projection into a homogeneous `List<Expr<Text>>`
literal spread into each SELECT list:

```sql
-- Spread two same-typed VARCHAR columns from raw_customers
SELECT order_id, customer_id, ...[customer_name, country], order_date, status, amount
FROM ...
```

The smelt LSP will show `List<Expr<TEXT>>` on hover for the literal and emit a helpful
diagnostic if you mis-shape it — for example `MetaListHeterogeneous` if the
elements don't all share a common type, or `MetaSpreadInForbiddenPosition` if
you accidentally place the spread inside a WHERE clause instead of the SELECT
list.  This extension is not validated by `validate.py`; it is workflow practice
only.

## Optional: HOF pipeline and config vars (Phase B surface)

After the Phase A lift, try using Phase B HOFs and `smelt.config.var` together.
Add a `vars:` block to your `smelt.yml`:

```yaml
vars:
  min_revenue: 50
```

Then use `smelt.config.var` in a model to read it at compile time, and compose
HOFs with the pipe operator to filter and map a list of amounts inline:

```sql
-- Filter out low-revenue orders, then double each remaining amount for illustration
SELECT [10, 75, 200, 30, 120]
         |> filter(fn x => x > smelt.config.var('min_revenue'))
         |> map(fn x => x * 2)
```

You can also fold a boolean column list with `and_all` or `or_any` reducers:

```sql
-- True if every order in a hardcoded set is above the threshold
SELECT reduce([75, 200, 120], and_all)
```

Wait — `and_all` expects `List<Boolean>`, not `List<Numeric>`. The LSP will
surface `ReducerInputTypeMismatch`. Fix it by adjusting the list to booleans:

```sql
SELECT reduce([true, true, false], or_any)
```

Key Phase B diagnostics to watch for:

- `ReducerInputTypeMismatch` — list element type doesn't match the reducer's expected input
- `ReducerEmptyNoIdentity` — `reduce([], union_all)` where `union_all` has no identity
- `HofExpectsLambda` — passing a bare name instead of `fn x => ...` to `map` or `filter`
- `ConfigVarNotFound` — referencing a `vars:` key that doesn't exist in `smelt.yml`
- `PipeRhsNotCall` — `list |> 3 + 4` (RHS of `|>` must be a call expression)

This extension is not validated by `validate.py`; it is workflow practice only.

## Optional: column reflection with smelt.columns_of (Phase C surface)

After the Phase B extension, try using Phase C column reflection to generate
per-column expressions automatically from a model's schema.

First, add a schema YAML for `raw_orders` under `models/sources/raw/raw_orders.yml`
that declares `amount` and `customer_id` as numeric columns:

```yaml
description: Raw orders
columns:
  - name: order_id
    type: INTEGER
  - name: customer_id
    type: INTEGER
  - name: order_date
    type: DATE
  - name: status
    type: VARCHAR
  - name: amount
    type: DOUBLE
```

Then write a function that uses `smelt.columns_of` and a HOF pipeline to wrap
every numeric column in `COALESCE`:

```sql
-- functions/coalesce_numeric.sql
smelt.define coalesce_numeric(t: TableExpr) -> SelectItems<Scalar, t> AS (
    smelt.columns_of(t)
      |> filter(fn c => c.is_numeric)
      |> map(fn c => COALESCE(c.name, 0))
)
```

And call it in a model using the spread form:

```sql
-- models/stg_orders_safe.sql
SELECT
    order_id,
    status,
    order_date,
    ...smelt.functions.coalesce_numeric(smelt.stg_orders)
FROM smelt.stg_orders
```

Phase C diagnostics to watch for when experimenting:

- `ColumnsOfRequiresTableExpr` — `smelt.columns_of(42)` where the argument is not a model reference
- `ColumnsOfNamedArgument` — `smelt.columns_of(t => orders)` uses an unsupported named argument
- `ColumnsOfUnresolvableSchema` — the model passed to `smelt.columns_of` has no resolvable schema
- `ColumnRefFieldUnknown` — `c.label` where `label` is not in the ColumnRef field set `{name, type, is_numeric}`

This extension is not validated by `validate.py`; it is workflow practice only.

## Optional: wide reflection with smelt.models.with_tag (Phase D surface)

After the Phase C extension, try using Phase D wide reflection to introspect
the workspace and iterate over models by tag.

First, tag two of your models with a `cohort` tag in their YAML frontmatter:

```sql
---
tags: [cohort]
---
SELECT ...
```

Then write a model that maps over all cohort models to collect their names:

```sql
-- models/cohort_inventory.sql
-- smelt.models.with_tag('cohort') returns List<ModelRef> — all models tagged
-- 'cohort' in workspace-relative path order.
-- map projects each ModelRef to its name field (Text).
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)
```

You can also explore `smelt.models.all()` (all workspace models) and field
projection:

```sql
-- All model paths in the workspace
SELECT map(smelt.models.all(), fn m => m.path)

-- Column list for every cohort model (m.columns is equivalent to smelt.columns_of(m))
SELECT map(smelt.models.with_tag('cohort'), fn m => m.columns)
```

Phase D rules to remember:
- `with_tag` takes exactly one positional Text literal: `with_tag('my-tag')`. Named
  arguments (`tag => 'cohort'`) emit `WithTagNamedArgument`; a runtime expression
  like `UPPER('cohort')` emits `WithTagRequiresText`.
- `all()` takes no arguments. Any argument emits `WideReflectionUnexpectedArgument`.
- ModelRef fields are exactly `{path, name, tags, columns}`. Any other field name
  emits `ModelRefFieldUnknown`.
- You do NOT need `m.table_expr` to use a ModelRef as a TableExpr — the subtyping
  lift `ModelRef <: TableExpr` is automatic.

Phase D diagnostics to watch for:

- `WithTagRequiresText` — `with_tag(42)` or `with_tag(UPPER('x'))` — argument must be a compile-time literal
- `WithTagNamedArgument` — `with_tag(tag => 'cohort')` — use positional syntax
- `WideReflectionUnknownAccessor` — `smelt.models.bogus()` — only `with_tag` and `all` are valid
- `WideReflectionUnexpectedArgument` — `smelt.models.all(42)` — `all` takes no arguments
- `ModelRefFieldUnknown` — `m.materialization` — field is not in the closed set

This extension is not validated by `validate.py`; it is workflow practice only.
