# Quickstart

This guide walks you through creating your first smelt project.

## 1. Create a project

```bash
mkdir my-project && cd my-project
```

Create a `smelt.yml` configuration file:

```yaml
name: my-project
targets:
  dev:
    type: duckdb
    database: dev.duckdb
    schema: main
```

## 2. Write a model

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

## 3. Run your models

```bash
# Preview what will be executed
smelt run --dry-run --verbose

# Execute all models
smelt run

# Show query results
smelt run --show-results
```

## 4. Set up your editor

Install the [smelt VSCode extension](../guide/editor-setup.md) for syntax highlighting, diagnostics, and go-to-definition support.
