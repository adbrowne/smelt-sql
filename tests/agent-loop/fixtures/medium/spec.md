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

Call them in your models as `smelt.functions.<path>.<name>(...)`. For example,
`smelt.functions.revenue.safe_revenue(o.amount)` if the file is
`functions/revenue.sql`.

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

Run `python ../validate.py` from inside your project directory. It checks both the
output table contents **and** that a `functions/` directory with at least one
`smelt.define` declaration exists and is called from your models.

## Hints

- The skill at `<run_dir>/skill.md` covers the project skeleton, model format, and CLI flags. Read it first.
- `smelt docs show concepts/functions` explains the `smelt.define` syntax and call paths.
- `smelt docs show getting-started/quickstart` gives the minimum project skeleton.
- Build the seeds → one staging model → `smelt build` first, then layer in the functions.
- After defining a function, run `smelt build --show-plan models/stg_orders.sql` to confirm the
  call expands correctly before doing a full build.
- `smelt.functions.<path>.<name>` — the path mirrors the directory and file structure under
  `functions/`. A function `safe_revenue` in `functions/revenue.sql` is called as
  `smelt.functions.revenue.safe_revenue(...)`.
