# Pipe Syntax

Pipe syntax lets you write a query as a linear chain of transformation stages, one per line. Instead of writing a single `SELECT` with deeply nested clauses, you start from a table and push it through a sequence of `|>` stages — each stage consuming the output of the one before it.

```sql
FROM orders
|> WHERE status = 'paid'
|> AGGREGATE sum(amount) AS revenue GROUP BY customer_id
|> WHERE revenue > 1000
|> ORDER BY revenue DESC
|> LIMIT 10
```

The key property is that **scope flows forward**: each stage sees exactly the columns produced by the previous stage, and nothing else. Aliases defined in one stage are immediately available in the next. There is no ambiguity about which columns are visible where.

## Pipe query form

A pipe query begins with a bare `FROM` clause (no leading `SELECT`) followed by at least one `|>` stage:

```sql
FROM <table>
|> <operator> …
|> <operator> …
```

A pipe query produces a relation just like a regular `SELECT` does, and can be used anywhere a query is valid — as a model body, inside a CTE, or as a subquery.

A pipe query may also begin with a `WITH` clause; the CTEs are in scope for the FROM-first body:

```sql
WITH recent AS (
    FROM events
    |> WHERE ts > current_date - interval '7 days'
)
FROM recent
|> AGGREGATE count(*) AS n GROUP BY source
```

## Operators

| Operator | Form | Effect |
|---|---|---|
| *(entry)* | `FROM <table>` | Table source: a table, `smelt.<path>`, a parenthesised subquery, or a join chain. Same `FROM` grammar as a SELECT. |
| `WHERE` | `\|> WHERE <predicate>` | Filter rows. Before any aggregation this is a `WHERE`; after aggregation it is a `HAVING`; after a window column it is a `QUALIFY`. |
| `SELECT` | `\|> SELECT <expr> [AS <alias>], …` | Project to exactly the listed columns. |
| `EXTEND` | `\|> EXTEND <expr> AS <alias>, …` | Append computed columns, keeping all existing columns. |
| `SET` | `\|> SET <col> = <expr>, …` | Replace the value of existing columns in place. |
| `DROP` | `\|> DROP <col>, …` | Remove the named columns, keep the rest. |
| `RENAME` | `\|> RENAME <old> AS <new>, …` | Rename columns, keeping all others. |
| `AS` | `\|> AS <alias>` | Give the intermediate table a range-variable alias. |
| `AGGREGATE` | `\|> AGGREGATE <agg_expr> [AS <alias>], … [GROUP BY <group_expr> [AS <alias>], …]` | Group and aggregate. Output columns are grouping keys first, then aggregates. Omit `GROUP BY` for a full-table aggregation (one output row). |
| `ORDER BY` | `\|> ORDER BY <expr> [ASC\|DESC] [NULLS …], …` | Order rows. |
| `LIMIT` | `\|> LIMIT <n> [OFFSET <m>]` | Limit the number of rows returned. |
| `JOIN` | `\|> [INNER\|LEFT\|RIGHT\|FULL\|CROSS] JOIN <table> [ON <cond> \| USING (<cols>)]` | Join; the pipe input is always the left side. |
| set ops | `\|> {UNION\|INTERSECT\|EXCEPT} {ALL\|DISTINCT} (<query>) [, (<query>)…]` | Set operations, left-folded across multiple operands. |
| `DISTINCT` | `\|> DISTINCT` | Deduplicate rows. |

## Scope rules

Each stage's visible columns are exactly the output of the previous stage:

- **`EXTEND`** adds columns. All prior columns plus the new ones are in scope for the next stage.
- **`AGGREGATE`** collapses scope to grouping keys and aggregate outputs. Columns from before the aggregation are no longer visible.
- **`DROP`** removes the named columns. The rest remain.
- **`RENAME`** renames a column. The new name is in scope; the old name is not.
- **`SET`** replaces a column's value in place. The column name stays the same.
- **`SELECT`** replaces the scope with exactly the listed columns.
- **`JOIN`** extends scope with the right table's columns.
- **`WHERE`**, **`ORDER BY`**, **`LIMIT`**, and **`DISTINCT`** pass the scope through unchanged.

A `|> WHERE` that follows an aggregation is automatically interpreted as `HAVING` — it filters on the aggregate output:

```sql
FROM orders
|> AGGREGATE sum(amount) AS revenue GROUP BY customer_id
|> WHERE revenue > 1000
```

A `|> WHERE` that follows a window column is interpreted as `QUALIFY`:

```sql
FROM events
|> EXTEND row_number() OVER (PARTITION BY user_id ORDER BY event_time) AS rn
|> WHERE rn = 1
```

## Examples

### Basic filter and projection

The canonical pipe query for filtering and projecting:

```sql
FROM orders
|> WHERE status = 'paid'
|> SELECT customer_id, amount
|> ORDER BY amount DESC
```

### Adding a computed column

`EXTEND` keeps all columns and appends the new one:

```sql
FROM nums
|> EXTEND n * 2 AS doubled
```

### Grouping and aggregation

```sql
FROM sales
|> AGGREGATE sum(amount) AS revenue GROUP BY customer_id
```

For a full-table total (no grouping):

```sql
FROM sales
|> AGGREGATE sum(amount) AS total
```

### Two-stage aggregation

Aggregation stages can be chained. Each level wraps the previous as a subquery:

```sql
FROM sales
|> AGGREGATE sum(amount) AS city_total GROUP BY region, city
|> AGGREGATE count(*) AS city_count GROUP BY region
```

### Join

The pipe input is the left side of the join:

```sql
FROM emps
|> JOIN depts ON emps.dept_id = depts.dept_id
|> ORDER BY name
```

All join variants are supported:

```sql
FROM t |> INNER JOIN s ON t.id = s.id
FROM t |> LEFT JOIN s ON t.id = s.id
FROM t |> RIGHT JOIN s ON t.id = s.id
FROM t |> FULL JOIN s ON t.id = s.id
FROM t |> CROSS JOIN s
FROM t |> JOIN s USING (id)
```

### Set operations

```sql
FROM emps
|> UNION ALL (SELECT * FROM extra_emps)
|> ORDER BY name
```

Multiple operands are left-folded:

```sql
FROM t
|> UNION ALL (SELECT * FROM u), (SELECT * FROM v)
```

### Deduplication

```sql
FROM t
|> SELECT a
|> DISTINCT
```

## Where a pipe query may appear

### As a model body

A `.sql` model whose body is a FROM-first pipe query:

```sql
-- @materialization: view
FROM smelt.raw_events
|> WHERE event_type = 'click'
|> SELECT user_id, event_time
|> ORDER BY event_time DESC
|> LIMIT 100
```

All model frontmatter options (`materialization`, `incremental`, `tags`, and so on) apply unchanged.

### As a CTE body

A pipe query can be used as the body of a `WITH` CTE. The outer query is standard SQL:

```sql
-- @materialization: view
WITH clicks AS (
    FROM smelt.raw_events
    |> WHERE event_type = 'click'
    |> SELECT user_id, event_time
)
SELECT user_id, COUNT(*) AS click_count
FROM clicks
GROUP BY user_id
```

### As a subquery

A pipe query can be parenthesised anywhere a subquery is valid:

```sql
-- @materialization: view
SELECT user_id, event_time
FROM (
    FROM smelt.raw_events
    |> WHERE event_type = 'click'
) filtered
```

## Lowering to standard SQL

Pipe queries lower to standard SQL before reaching the backend. The lowered SQL computes the same relation as the pipe query; backends receive standard SQL regardless of whether the original model used pipe syntax or not.

Contiguous stages that fit one query level collapse into a single `SELECT`. Stages that require a new query level (a second `AGGREGATE`, a `WHERE` that follows a `SELECT` whose aliases would be invisible in a same-level `WHERE`) wrap the prior query as a subquery automatically.

This lowering is transparent — you author in pipe style, smelt handles the translation.

## Unsupported operators

The following pipe operators are not supported. Using one is a hard error, not a silent no-op:

| Operator | Why |
|---|---|
| `PIVOT` / `UNPIVOT` | Output columns depend on data values and cannot be determined at compile time. |
| `WINDOW name AS (…)` | The named-window form is not supported. Window functions inside `SELECT`/`EXTEND` expressions work normally. |
| `CALL` | Table-valued function piping has no end-to-end smelt support. |
| `TABLESAMPLE` | Sampling is available on a `FROM` table reference only, not as a stage. |
| `ASSERT` | Row-level runtime assertions have no smelt equivalent. |

## Diagnostics

| Code | When it fires | Example message |
|---|---|---|
| `PipeUnknownOperator` | `\|>` followed by an unrecognised keyword | `unknown pipe operator 'FROBNICATE'` |
| `PipeOperatorUnsupported` | A known-but-unsupported operator (`PIVOT`, `WINDOW`, etc.) | `pipe operator 'PIVOT' is not supported — output columns depend on data values` |
| `PipeStageMalformed` | A stage whose body does not parse | `malformed 'WHERE' pipe stage` |
