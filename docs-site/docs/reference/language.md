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

### smelt.models

Reference another model in the project:

```sql
FROM smelt.models.model_name
FROM smelt.models.model_name(filter => condition, limit => n)
```

### smelt.sources

Reference an external source table declared as a per-entity `.yml` under `paths:`:

```sql
FROM smelt.sources.source.table
```

### smelt.define — user-defined functions

Declare a reusable SQL fragment with optional type annotations. Files live in `functions/`.

```sql
-- Tier 1 (unannotated)
smelt.define add_one(x) AS (x + 1)

-- Tier 2 (parameters annotated)
smelt.define safe_divide(
  numerator: Expr<Numeric>,
  denominator: Expr<Numeric>
) AS (
  CASE WHEN denominator = 0 OR denominator IS NULL
    THEN NULL
    ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
  END
)

-- Tier 3 (fully annotated, return type verified)
smelt.define safe_divide(
  numerator: Expr<Numeric>,
  denominator: Expr<Numeric>
) -> Expr<Double> AS (
  CASE WHEN denominator = 0 OR denominator IS NULL
    THEN NULL
    ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE)
  END
)
```

See the [Functions guide](../guide/functions.md) for the full type annotation language, fragment sorts (`TableExpr`, `SelectItems`), and `PASSING` clauses.

### smelt.functions.* — calling user-defined functions

```sql
-- Positional arguments
SELECT smelt.functions.safe_divide(revenue, cost) AS margin FROM smelt.models.orders

-- Named arguments
SELECT * FROM smelt.functions.sessionize(
  smelt.models.events,
  user_col => user_id,
  ts_col   => event_time
)

-- PASSING clause for fragment parameters
SELECT *
FROM smelt.functions.session_rollup(smelt.models.events, user_id, event_time)
PASSING metrics AS (COUNT(*) AS events, SUM(amount) AS total)
```

### smelt.extern — external function declarations

Declare a backend-native function so smelt can type-check call sites:

```sql
smelt.extern regex_match(
  text: Expr<Text>,
  pattern: Expr<Text>
) -> Expr<Boolean>
```

### smelt.as_struct() — struct packing

Bundle columns from a table alias into a struct value:

```sql
SELECT
  smelt.as_struct(o EXCEPT customer_id) AS order_data,
  smelt.as_struct(c EXCEPT customer_id) AS customer_data
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.customer_id
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

### Labelling rollup rows

`CUBE`, `ROLLUP`, and `GROUPING SETS` produce extra "subtotal" rows where the
grouped-out columns are returned as `NULL`. Use `GROUPING()` to detect those
rollup rows and label them with a sentinel value:

```sql
SELECT
  CASE WHEN GROUPING(category) = 1 THEN 'ALL' ELSE category END AS category,
  CASE WHEN GROUPING(region)   = 1 THEN 'ALL' ELSE region   END AS region,
  SUM(amount) AS total
FROM smelt.models.sales
GROUP BY CUBE(category, region)
```

!!! warning "Pitfall: do not use `COALESCE(col, 'ALL')` for rollup labels"
    `COALESCE(category, 'ALL')` looks like a shorter way to write the same
    thing, but it is wrong whenever `category` is nullable. A real `NULL` in
    the source data and a CUBE-rolled-up `NULL` both collapse to `'ALL'`,
    producing two rows that look like the grand total but are actually
    different aggregations. `GROUPING(col) = 1` distinguishes "this column was
    rolled up by CUBE" from "this column happens to be NULL in the data", so
    real NULLs stay as NULL (or can be labelled separately) and only true
    rollup rows get the sentinel.

    `COALESCE(col, 'ALL')` is only safe when `col` is declared `NOT NULL` at
    the source.

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

## Aggregate result types

smelt assigns canonical return types to aggregates so the same model writes the same output schema on every backend — `SUM(integer)` gives you `BIGINT` whether you target DuckDB or PostgreSQL, even though the engines disagree natively. Knowing the exact widening rules matters when a downstream column or test expects a specific type; `COUNT(*)` is a frequent surprise because it returns `BIGINT` rather than `INTEGER`.

| Aggregate | Argument type | Result type | Nullable |
|---|---|---|---|
| `COUNT(*)`, `COUNT(expr)` | any | `BIGINT` | no |
| `SUM(x)` | `SMALLINT`, `INTEGER`, `BIGINT` | `BIGINT` | yes |
| `SUM(x)` | `FLOAT`, `DOUBLE` | `DOUBLE` | yes |
| `SUM(x)` | `DECIMAL(p, s)` | `DECIMAL(38, s)` | yes |
| `AVG(x)` | any numeric | `DOUBLE` | yes |
| `MIN(x)`, `MAX(x)` | any | same as `x` | yes |

### Notes

- **`SUM(DECIMAL)` widens precision to 38.** Real pipelines that accumulate ~1e6 rows of `DECIMAL(10, 2)` overflow precision 10 quickly; smelt mirrors DuckDB's widen-to-38 to avoid silent corruption. Scale is preserved.
- **`COUNT` is non-null; everything else is nullable.** Other aggregates can return `NULL` when the input group is empty (common with `LEFT JOIN`-fed `GROUP BY`). To substitute a default, wrap in `COALESCE`:

    ```sql
    SELECT
      c.customer_id,
      COALESCE(SUM(o.amount), 0) AS lifetime_spend
    FROM smelt.models.customers AS c
    LEFT JOIN smelt.models.orders AS o USING (customer_id)
    GROUP BY c.customer_id
    ```

- **Cast `COUNT` if a downstream column expects `INTEGER`.** `CAST(COUNT(*) AS INTEGER)` is safe up to `2^31 - 1` rows; above that, leave it as `BIGINT`.

See [`docs/specs/types.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/specs/types.md) §5 for the normative rules and [`docs/type_semantics.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/type_semantics.md) for backend divergence notes.

## Multi-dialect features

These features are parsed in smelt SQL and rewritten to target-specific syntax:

- **QUALIFY** — window function filtering (DuckDB/Spark origin)
- **Lambda expressions** — `x -> x + 1` for array functions
- **PIVOT / UNPIVOT** — table rotation
- **Array subscript** — `arr[1]` notation
- **DATE literals** — `DATE '2024-01-01'` normalization
