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

A model file contains at most one query body. Any content after it — a second `SELECT`, stray tokens, or the tail of an unsupported construct — is an error, surfaced as a `trailing-top-level-content` diagnostic; it is never silently ignored.

## smelt extensions

### smelt.&lt;path&gt;

Reference another model in the project:

```sql
FROM smelt.model_name
FROM smelt.model_name(filter => condition, limit => n)
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
SELECT smelt.functions.safe_divide(revenue, cost) AS margin FROM smelt.orders

-- Named arguments
SELECT * FROM smelt.functions.sessionize(
  smelt.events,
  user_col => user_id,
  ts_col   => event_time
)

-- PASSING clause for fragment parameters
SELECT *
FROM smelt.functions.session_rollup(smelt.events, user_id, event_time)
PASSING metrics AS (COUNT(*) AS events, SUM(amount) AS total)

-- Struct spread: project all fields of an Expr<Struct<{...}>> return as columns
SELECT smelt.functions.parse_event_payload(payload).*
FROM smelt.sources.raw.events
```

When a function declares `-> Expr<Struct<{field1: Type1, field2: Type2, …}>>`, the `.*` suffix expands the struct fields into individual columns in the model's output schema. Each field becomes a separately named column with its declared type. This expansion is visible to downstream models and the LSP — hover, diagnostics, and completions all reflect the struct's declared fields.

Row-polymorphic functions (`Struct<{…, ..r}>`) expand declared fields plus any extras bound from the call-site argument's schema.

An unrecognized type name in any struct field position of a function annotation is an `InvalidFunctionTypeRef` error anchored at the declaration. For example, `-> Expr<Struct<{a: Integer, b: Bogus}>>` where `Bogus` is not a known type emits `InvalidFunctionTypeRef` at the return-type annotation. The error fires at the declaration so that callers projecting the struct's fields observe the resulting `Unknown` column as a downstream consequence rather than receiving a separate call-site diagnostic.

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

An optional **column list** after the CTE name rebinds the inner SELECT's column types to the declared names, positionally:

```sql
-- Inner SELECT columns are renamed to (a, b) while keeping their types
WITH cte(a, b) AS (SELECT CAST(1 AS INTEGER), CAST(2.0 AS DOUBLE))
SELECT a, b FROM cte
-- a: Integer, b: Double
```

When the column list is omitted, the inner SELECT's own aliases are used unchanged.

When the declared column count does not match the inner SELECT's actual column count, smelt emits `AliasColumnArityMismatch` anchored at the column-list span. Alias names are applied positionally up to whichever list is shorter; any remaining columns retain their inferred names:

```sql
-- Error: alias list has 1 name but SELECT produces 2 columns
WITH cte(a) AS (SELECT CAST(1 AS INTEGER), CAST(2 AS INTEGER))
SELECT a FROM cte
-- AliasColumnArityMismatch at (a)
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
FROM smelt.sales
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

### VALUES-derived tables

A `VALUES` clause in a derived-table position produces a typed schema. smelt infers each column's type as the least upper bound (LUB) of the corresponding elements across all rows, following the numeric promotion chain (`SmallInt < Integer < BigInt < Decimal < Double`):

```sql
-- Alias column list provides names; types are inferred from the rows
SELECT id, region, created_at
FROM (
    VALUES
        (1, 'us-west-2', CAST('2024-01-01' AS TIMESTAMP)),
        (2, 'eu-west-1', CAST('2024-01-02' AS TIMESTAMP))
) AS t(id, region, created_at)
-- id: SMALLINT, region: TEXT, created_at: TIMESTAMP

-- Multi-row promotion: Integer + Double → Double
SELECT x FROM (VALUES (CAST(1 AS INTEGER)), (CAST(2.0 AS DOUBLE))) AS t(x)
-- x: Double

-- Without an alias column list, columns are named col1, col2, …
SELECT col1, col2 FROM (VALUES (1, 2)) AS t
```

When the alias column list has a different length from the number of VALUES columns, smelt emits `AliasColumnArityMismatch` anchored at the column-list span:

```sql
-- Error: alias list has 1 name but VALUES produces 2 columns per row
SELECT a FROM (VALUES (CAST(1 AS INTEGER), CAST(2 AS INTEGER))) AS t(a)
-- AliasColumnArityMismatch at (a)
```

## Type casting

```sql
CAST(x AS INTEGER)
x::INTEGER          -- PostgreSQL-style
TRY_CAST(x AS DATE) -- Returns NULL on failure
```

### Numeric literal forms

smelt accepts plain integer and decimal literals (`1`, `1.5`), including scientific notation (`1e8`, `1.5e-3`). A numeric literal immediately followed by letters or an underscore with no separating space — `0x1F`, `1_000_000` — is not accepted as a single literal; it produces a parse error rather than being silently reinterpreted (e.g. as `0` implicitly aliased to `x1F`). Write a space before an intended alias (`1 x`) or drop the digit-separator/hex-prefix form.

`E'...'` (escape string) and `B'...'` (bit-string-shaped) prefixed string literals lex as ordinary string literals.

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
    FROM smelt.customers AS c
    LEFT JOIN smelt.orders AS o USING (customer_id)
    GROUP BY c.customer_id
    ```

- **Cast `COUNT` if a downstream column expects `INTEGER`.** `CAST(COUNT(*) AS INTEGER)` is safe up to `2^31 - 1` rows; above that, leave it as `BIGINT`.

See [`docs/specs/types.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/specs/types.md) §5 for the normative rules and [`docs/type_semantics.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/type_semantics.md) for backend divergence notes.

## String collation

smelt supports the `COLLATE` clause for explicit string collation: `expr COLLATE collation_name`.

In **portable models** (no `engine:` declaration), only the binary collation is allowed.
Binary collation names are case-insensitive and accepted on all target backends:

| Collation name | Notes |
|---|---|
| `"C"` or `POSIX` | ISO/ANSI byte-order comparison (PostgreSQL/DuckDB convention) |
| `BINARY` | DuckDB default byte-order comparison |
| `UTF8_BINARY` | Spark default byte-order comparison |

Binary collation is a **no-op for type inference**: `expr COLLATE "C"` returns the same type as `expr`.

```sql
-- ok: binary collation is portable
SELECT name COLLATE "C" AS sorted_name FROM t
SELECT name COLLATE BINARY AS sorted_name FROM t
```

Using any **non-binary collation** (case-insensitive, locale-aware, accent-insensitive) in a portable model is a `NonPortableCollation` error. The comparison degrades to `Unknown` type to prevent silent cross-engine divergence:

```sql
-- error: non-portable collation — use COLLATE "C" or remove the clause
SELECT name COLLATE NOCASE AS sorted_name FROM t
```

To use a non-binary collation, declare an engine on the model so smelt can emit engine-specific SQL.

**Binary string comparisons and grouping are stable across all target engines.**
Under binary (byte-wise) collation, the following operations produce identical results on DuckDB,
Spark, and PostgreSQL regardless of the database's locale setting:

- Equality and ordering (`=`, `<`, `<=`, `>`, `>=`)
- `GROUP BY` and `DISTINCT` on string columns
- `ORDER BY` on string columns
- `MIN` and `MAX` over string columns

This means a portable smelt model that groups, sorts, or deduplicates strings produces the
same rows in the same order on every engine — no cross-engine divergence, no silent locale
differences.

## Multi-dialect features

These features are parsed in smelt SQL and rewritten to target-specific syntax:

- **QUALIFY** — window function filtering (DuckDB/Spark origin)
- **Lambda expressions** — `x -> x + 1` for array functions
- **PIVOT / UNPIVOT** — table rotation
- **Array subscript** — `arr[1]` notation
- **DATE literals** — `DATE '2024-01-01'` normalization
