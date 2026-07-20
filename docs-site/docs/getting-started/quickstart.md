# Quickstart

This guide walks you through creating your first smelt project.

## 1. Scaffold a project

```bash
smelt init my-project && cd my-project
```

`smelt init` writes a minimal, working project to `my-project/`: a `smelt.yml`, a `models/` directory with one example model (`orders_summary.sql`), a seed CSV (`seeds/raw_orders.csv`), and a `.gitignore` excluding `.smelt/` and the database file. It takes no interactive prompts — every file is a fixed template — and refuses to run (exit code `2`) against a directory that already has a `smelt.yml`, so it's always safe to run in a fresh directory. Run `smelt build` right away and it succeeds with no further edits; the rest of this guide extends that scaffold.

The generated `smelt.yml` looks like this:

```yaml
name: my-project
version: 1
paths:
  - models
  - seeds
targets:
  dev:
    type: duckdb
    database: dev.duckdb
    schema: main
```

!!! note "Supported `smelt.yml` keys at a glance"
    | Key | Required | Default |
    |---|---|---|
    | `name` | yes | — |
    | `version` | no | `1` |
    | `targets` | yes | — |
    | `paths` | no | `["models"]` |
    | `default_materialization` | no | `"view"` |
    | `unstable_schema` | no | `false` |

    Unknown keys produce a warning, not an error. See the [smelt.yml reference](../reference/smelt-yml.md) for the full per-key contract.

## 2. Add more seed data

The scaffold already has `seeds/raw_orders.csv`:

```csv
order_id,order_date,customer_id,amount
1,2025-01-01,100,29.99
2,2025-01-01,101,49.99
3,2025-01-02,100,19.99
```

Add a second seed, `seeds/raw_customers.csv`:

```csv
customer_id,name,country
100,Alice,AU
101,Bob,NZ
```

Seeds become tables addressed as `smelt.<name>` (e.g. `smelt.raw_orders`, `smelt.raw_customers`). Seeds and SQL models share the same flat namespace — there is no `smelt.models.*` prefix.

## 3. Write another model

The scaffold's `models/orders_summary.sql` is already a simple aggregate model:

```sql
-- models/orders_summary.sql
---
name: orders_summary
materialization: table
---
SELECT
  DATE(order_date) AS order_day,
  COUNT(*) AS order_count,
  SUM(amount) AS total_amount
FROM smelt.raw_orders
GROUP BY 1
```

You can also join seeds together — for example, to enrich orders with customer details:

```sql
-- models/stg_orders.sql
---
name: stg_orders
materialization: table
---
SELECT
  o.order_id,
  o.order_date,
  o.customer_id,
  o.amount,
  c.name AS customer_name,
  c.country
FROM smelt.raw_orders o
LEFT JOIN smelt.raw_customers c ON o.customer_id = c.customer_id
```

If a seed column name doesn't match what your model needs, alias it in the model — seed column names are locked to the CSV header row and cannot be renamed in `smelt.yml`.

You can also reference SQL models from other models — seeds and models share the same flat `smelt.<name>` namespace. A mart that rolls up from a staging model looks like this:

```sql
-- models/mart_customers.sql
---
name: mart_customers
materialization: table
---
SELECT
  c.customer_id,
  c.name AS customer_name,
  COALESCE(SUM(o.amount), 0.0) AS total_revenue
FROM smelt.raw_customers c
LEFT JOIN smelt.stg_orders o ON c.customer_id = o.customer_id
GROUP BY c.customer_id, c.name
```

Starting from the dimension seed (`raw_customers`) and LEFT JOINing the staging model (`stg_orders`) ensures every customer appears in the output, even those with no orders.

## 4. Run your models

```bash
# Execute all models (seeds + models)
smelt build

# Show query results
smelt build --show-results

# Show compiled SQL for each model
smelt build --verbose
```

## 5. Set up your editor

Install the [smelt VSCode extension](../guide/editor-setup.md) for syntax highlighting, diagnostics, and go-to-definition support.

## Next steps

- [How smelt Works](../concepts/how-it-works.md) -- understand the logical/physical separation
- [Incremental Models](../guide/incremental-models.md) -- process only new data
- [Deployment](../guide/deployment.md) -- run smelt outside your laptop (CI, containers, scheduled jobs)
- [Orchestration](../guide/orchestration.md) -- run smelt under cron, Airflow, or another scheduler
- [CLI Reference](../reference/cli.md) -- full command documentation
