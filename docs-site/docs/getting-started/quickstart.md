# Quickstart

This guide walks you through creating your first smelt project.

## 1. Create a project

```bash
mkdir my-project && cd my-project
```

Create a `smelt.yml` configuration file:

```yaml
name: my-project
version: 1
targets:
  dev:
    type: duckdb
    database: dev.duckdb
    schema: main
```

## 2. Add seed data

Create a `seeds/` directory with a CSV file:

```bash
mkdir seeds
```

Create `seeds/raw_orders.csv`:

```csv
order_id,order_date,customer_id,amount
1,2025-01-01,100,29.99
2,2025-01-01,101,49.99
3,2025-01-02,100,19.99
```

Seeds become tables that can be referenced with `smelt.ref()`.

## 3. Write a model

Create a `models/` directory and add your first SQL model:

```bash
mkdir models
```

```sql
-- models/orders_summary.sql
---
name: orders_summary
materialization: table
---

SELECT
  DATE(order_date) as order_day,
  COUNT(*) as order_count,
  SUM(amount) as total_amount
FROM smelt.ref('raw_orders')
GROUP BY 1
```

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
- [CLI Reference](../reference/cli.md) -- full command documentation
