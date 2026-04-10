# Language Reference

smelt SQL is a logical SQL superset — PostgreSQL base with cherry-picked features from DuckDB and Spark. Models are compiled to target-specific SQL for execution.

## SELECT statement

Standard SQL SELECT with all common clauses:

```sql
SELECT [DISTINCT] columns
FROM table_references
[WHERE condition]
[GROUP BY expressions]
[HAVING condition]
[QUALIFY condition]
[ORDER BY expressions]
[LIMIT n]
[OFFSET n]
```

## smelt extensions

### smelt.ref()

Reference another model in the project:

```sql
FROM smelt.ref('model_name')
FROM smelt.ref('model_name', filter => condition, limit => n)
```

### smelt.source()

Reference an external source table defined in `sources.yml`:

```sql
FROM smelt.source('source.table')
```

## JOIN syntax

All standard JOIN types are supported:

```sql
FROM a
INNER JOIN b ON a.id = b.id
LEFT JOIN c USING (id)
RIGHT JOIN d ON a.key = d.key
FULL OUTER JOIN e ON a.id = e.id
CROSS JOIN f
```

## Window functions

```sql
SUM(amount) OVER (PARTITION BY user_id ORDER BY date ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
ROW_NUMBER() OVER (PARTITION BY group_col ORDER BY sort_col)
LAG(value, 1) OVER (ORDER BY date)
PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY val)
```

## Common Table Expressions

```sql
WITH
  cte1 AS (SELECT ...),
  cte2 AS (SELECT ... FROM cte1)
SELECT * FROM cte2
```

## Set operations

```sql
SELECT ... UNION ALL SELECT ...
SELECT ... INTERSECT SELECT ...
SELECT ... EXCEPT SELECT ...
```

## GROUP BY extensions

```sql
GROUP BY CUBE(a, b)
GROUP BY ROLLUP(a, b)
GROUP BY GROUPING SETS ((a, b), (a), ())
```

## Subqueries

```sql
-- Scalar subquery
SELECT (SELECT MAX(amount) FROM orders) as max_amount

-- EXISTS
WHERE EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id)

-- IN subquery
WHERE user_id IN (SELECT user_id FROM active_users)
```

## Type casting

```sql
CAST(x AS INTEGER)
x::INTEGER          -- PostgreSQL-style
TRY_CAST(x AS DATE) -- Returns NULL on failure
```

## Date/time extraction

```sql
EXTRACT(EPOCH FROM timestamp_col)   -- returns DOUBLE (Unix timestamp)
EXTRACT(YEAR FROM date_col)          -- returns BIGINT
EXTRACT(MONTH FROM timestamp_col)    -- returns BIGINT
EXTRACT(DAY FROM date_col)           -- returns BIGINT
EXTRACT(HOUR FROM timestamp_col)     -- returns BIGINT
EXTRACT(MINUTE FROM timestamp_col)   -- returns BIGINT
EXTRACT(SECOND FROM timestamp_col)   -- returns BIGINT
EXTRACT(DOW FROM date_col)           -- day of week, returns BIGINT
EXTRACT(DOY FROM date_col)           -- day of year, returns BIGINT
EXTRACT(QUARTER FROM date_col)       -- returns BIGINT
EXTRACT(WEEK FROM date_col)          -- returns BIGINT
```

`EXTRACT(EPOCH FROM ...)` returns a `DOUBLE` (floating-point Unix timestamp). All other fields return `BIGINT`.

## Multi-dialect features

These features are parsed in smelt SQL and rewritten to target-specific syntax:

- **QUALIFY** — window function filtering (DuckDB/Spark origin)
- **Lambda expressions** — `x -> x + 1` for array functions
- **PIVOT / UNPIVOT** — table rotation
- **Array subscript** — `arr[1]` notation
- **DATE literals** — `DATE '2024-01-01'` normalization
