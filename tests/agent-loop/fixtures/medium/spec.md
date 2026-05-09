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
