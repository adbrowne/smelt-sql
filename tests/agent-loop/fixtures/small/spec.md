# Small fixture: orders pipeline

You are building a smelt data project that turns raw orders and customer data into two reports for a small online shop. The project is the simplest end-to-end shape: seeds → staging → mart, no external sources, no incremental models.

## Inputs

Two seed CSVs are already prepared and copied into the `seeds/` directory of your project for you. **Do not regenerate them — use them as-is.**

- `seeds/raw_customers.csv` — 5 rows: `customer_id`, `name`, `country`
- `seeds/raw_orders.csv` — 10 rows: `order_id`, `customer_id`, `order_date`, `status`, `amount`

`status` is one of `shipped` or `cancelled`. `amount` is in dollars (decimal).

## Required outputs

The final DuckDB database must contain these tables:

### `stg_orders`
One row per order. Columns:

- `order_id` (INTEGER)
- `customer_id` (INTEGER)
- `customer_name` (VARCHAR) — joined from `raw_customers`
- `country` (VARCHAR) — joined from `raw_customers`
- `order_date` (DATE)
- `status` (VARCHAR)
- `amount` (DECIMAL or DOUBLE)

All 10 orders appear here, including cancelled ones.

### `int_orders_by_day`
Daily summary of **shipped orders only** (cancelled orders excluded). One row per `order_date` that has at least one shipped order. Columns:

- `order_date` (DATE)
- `order_count` (INTEGER)
- `total_revenue` (DECIMAL or DOUBLE)

### `mart_top_customers`
One row per customer (all 5 customers must appear, even if they have no shipped orders). Columns:

- `customer_id` (INTEGER)
- `customer_name` (VARCHAR)
- `total_revenue` (DECIMAL or DOUBLE) — lifetime revenue from **shipped** orders only; 0 for customers with no shipped orders.

## How to know you are done

Run `python ../validate.py` from inside your project directory. It connects to your DuckDB file and runs binary checks against the three tables above. The harness invokes `validate.py` automatically after your `smelt build`.

## Hints

- The skill at `<run_dir>/skill.md` covers the project skeleton, model file format, and CLI flags. Read it first.
- The CLI ships its own docs — `smelt docs list` to discover, `smelt docs show <topic>` to read.
- Build a minimal version (one seed → one staging model) first and verify with `smelt build` before adding the rest.
