# smelt

Modern data transformation framework.

smelt is a data transformation framework that separates **logical transformation definitions** from **physical execution planning**. Unlike dbt which uses Jinja templates, smelt parses and understands the semantics of your SQL, enabling:

- **Automatic incrementalization** — the framework analyzes what's safe to run incrementally
- **Cross-engine deployment** — split work across DuckDB, Spark, Postgres, etc.
- **Static analysis** — type checking, LSP support, and semantic validation
- **Rule-based optimization** — automatic rewrites with learning from history

## Quick Example

```sql
-- models/daily_revenue.sql
---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
tags: [revenue, daily]
---

SELECT
  order_date,
  customer_id,
  SUM(amount) as daily_revenue
FROM smelt.ref('orders')
WHERE order_date IS NOT NULL
GROUP BY 1, 2
```

```bash
# Run all models
smelt run

# Run incrementally (only process new data)
smelt run --event-time-start 2025-01-01 --event-time-end 2025-01-02

# Preview what would run
smelt run --dry-run --verbose
```

## Key Differentiators from dbt

| Aspect | dbt | smelt |
|--------|-----|-------|
| Model definition | Jinja templates | Parsed semantic models |
| Incrementalization | Manual `is_incremental()` | Automatic semantic analysis |
| Type checking | None (runtime errors) | Static analysis with LSP |
| Cross-engine | One target per project | Split work across engines |
| Optimization | Manual | Rule-based with learning |
