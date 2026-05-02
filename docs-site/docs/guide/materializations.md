# Materializations

A materialization controls how smelt persists the results of a model in the target database. There are four materialization types, each suited to different use cases.

## Materialization types

### view (default)

Creates a SQL view. No data is stored -- the query is re-evaluated each time the view is read.

```sql
CREATE VIEW user_events AS
  SELECT user_id, COUNT(*) as event_count
  FROM events GROUP BY 1;
```

Best for:

- Lightweight transforms and staging layers
- Models that are queried infrequently
- Keeping the database small during development

### table

Creates a physical table. Data is persisted and only recomputed when you run the model again.

```sql
CREATE TABLE daily_revenue AS
  SELECT date, SUM(amount) as revenue
  FROM transactions GROUP BY 1;
```

Best for:

- Frequently queried models
- Heavy aggregations you don't want to recompute on every read
- Models with many downstream dependents
- Incremental models (incremental requires `table` materialization)

### ephemeral

Not materialized at all. The model's SQL is inlined as a CTE (Common Table Expression) into every downstream model that references it.

Best for:

- Intermediate transformation steps that don't need to be queried directly
- Reducing the number of objects in your database
- Simple column renames or type casts

!!! warning
    Ephemeral models cannot have incremental configuration or target overrides. smelt will raise an error if you try to combine these.

### materialized_view

Creates a backend-managed materialized view. The database handles refresh scheduling and invalidation.

Best for:

- Backends that support materialized view refresh (PostgreSQL, Databricks)
- Cases where you want the database to manage the refresh lifecycle

!!! note
    Materialized views are refreshed atomically by the backend. Incremental configuration on a materialized view has no effect and will produce a warning.

## Setting materialization

There are three ways to set a model's materialization. They are listed here in order of precedence (highest to lowest):

### 1. YAML frontmatter in the SQL file

```sql
---
materialization: table
---

SELECT user_id, SUM(amount) as lifetime_value
FROM smelt.models.transactions
GROUP BY 1
```

### 2. models section in smelt.yml

```yaml
models:
  daily_revenue:
    materialization: table
  user_activity:
    materialization: table
  staging_users:
    materialization: ephemeral
```

### 3. default_materialization in smelt.yml

```yaml
default_materialization: view
```

If no materialization is specified anywhere, the default is `view`.

!!! tip
    A common pattern is to set `default_materialization: view` in `smelt.yml`, then override specific models to `table` where performance matters. This keeps development fast while ensuring production-critical models are materialized.

## Decision guide

| Scenario | Recommended |
|---|---|
| Staging layer / light transforms | `view` |
| Aggregations queried by BI tools | `table` |
| Intermediate step used by one downstream | `ephemeral` |
| Model with many downstream dependents | `table` |
| Incremental processing | `table` (required) |
| Database-managed refresh | `materialized_view` |
| Development / iteration | `view` |

## Changing materialization type

When you change a model from `view` to `table` (or vice versa), smelt automatically drops the existing object — regardless of its current type — before creating the new one. You do not need to manually drop the old view before running a model as a table.

```sql
-- Before: view (no frontmatter)
-- After: add frontmatter to change to table

---
materialization: table
---

SELECT ...
```

Run `smelt run --select my_model` and smelt will drop the view and create the table automatically.

## Further reading

- [Incremental Models](incremental-models.md) for time-partitioned incremental processing (requires `table`)
- [SQL Models](sql-models.md) for YAML frontmatter syntax
