# Testing

smelt lets you test your SQL models by defining mock input data and expected output rows
directly in SQL files, without needing a running database or executing your full pipeline.

## How it works

A test declares an assertion query that references the model(s) under test, provides mock
data for their dependencies via `PASSING` clauses, and states the expected output via an
`EXPECT` clause. When you run `smelt test`, smelt compiles the assertion query into a
standalone SQL query with mock data substituted for dependencies, executes it against an
in-memory DuckDB instance, and compares the actual output to the expected rows.

Tests are discovered the same way as models — by scanning the directories listed in `paths:`
in your `smelt.yml`. They can live in a dedicated `tests/` directory or co-located in
model files.

## smelt.test declarations

The primary way to write tests is with a `smelt.test` declaration. This keeps the query,
mock data, and expectations together in a single SQL-native form:

```sql
smelt.test daily_revenue_basic AS (
    SELECT order_date, total_revenue
    FROM smelt.daily_revenue
)
PASSING orders AS (
    {order_id: 1, amount: 100.0, order_date: '2024-01-15'},
    {order_id: 2, amount: 200.0, order_date: '2024-01-15'}
)
EXPECT (
    {order_date: '2024-01-15', total_revenue: 300.0}
)
```

The grammar is:

```
smelt.test <name> AS ( <select> )
  [ PASSING <dep> AS ( <rows> ) ]...
  EXPECT ( <rows> )
```

- **`<select>`** — the assertion query. It references the model(s) under test via
  `smelt.<path>`. There is no separate `model:` field; the model under test is
  determined by what the query references.
- **`PASSING <dep> AS ( <rows> )`** — mock data for one dependency. `<dep>` is the bare
  address path of the dependency (e.g. `orders` or `silver.orders`) — the `smelt.<path>`
  reference minus the leading `smelt.`. `<rows>` is a comma-separated list of record
  literals `{col: value, ...}`. Zero or more `PASSING` clauses are allowed.
- **`EXPECT ( <rows> )`** — required. The expected output rows as record literals.

Dependencies not named in any `PASSING` clause are replaced with empty CTEs (zero rows).
A `PASSING` clause that names a dependency the query does not actually reach is reported
as `UnknownTestInput` and fails the test — this catches typos that would otherwise
silently produce a false-green result.

### Record-literal value types

Each value in a record literal is automatically cast to the appropriate SQL type:

| Literal | SQL type | Example |
|---------|----------|---------|
| Integer | `INTEGER` | `42` |
| Float | `DOUBLE` | `3.14` |
| Decimal-shaped string (has `.`, no exponent) | `DECIMAL` | `'300.00'` |
| `'YYYY-MM-DD'` string | `DATE` | `'2024-01-15'` |
| `'YYYY-MM-DD HH:MM:SS'` string | `TIMESTAMP` | `'2024-01-15 10:00:00'` |
| Other string | `VARCHAR` | `'completed'` |
| Boolean | `BOOLEAN` | `true`, `false` |
| Null | `NULL` | `null` |

### Frontmatter knobs

A YAML frontmatter block can precede the `smelt.test` declaration to configure test behaviour:

```sql
---
test:
  check_order: true
  cases: 20
---
smelt.test check_revenue_rank AS (
    SELECT rank, user_id FROM smelt.revenue_report ORDER BY rank
)
PASSING ... 
EXPECT ...
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `check_order` | bool | `false` | If `true`, compare rows positionally (order matters). If `false`, compare as sets. |
| `cases` | integer | `10` | Number of iterations for property-based tests (see below). |

### Full-query tests

A full-query test inlines the referenced model's SQL and substitutes mock data for every
`smelt.<path>` dependency named in a `PASSING` clause:

```sql
smelt.test check_user_activity AS (
    SELECT user_id, total_events
    FROM smelt.user_activity
)
PASSING users AS (
    {user_id: 1, user_name: 'Alice', signup_date: '2024-01-01'},
    {user_id: 2, user_name: 'Bob', signup_date: '2024-02-01'}
)
PASSING events AS (
    {event_id: 1, user_id: 1, event_type: 'page_view'},
    {event_id: 2, user_id: 1, event_type: 'click'},
    {event_id: 3, user_id: 2, event_type: 'page_view'}
)
EXPECT (
    {user_id: 1, total_events: 2},
    {user_id: 2, total_events: 1}
)
```

### CTE-level tests with the `#` operator

Within a `smelt.test` body, you can target a specific CTE inside a model using the
`smelt.<model>#<cte>` syntax:

```sql
smelt.test daily_agg_rollup AS (
    SELECT day, revenue
    FROM smelt.daily_revenue#daily_agg
)
PASSING orders AS (
    {order_id: 1, amount: 100.0, order_date: '2024-01-01'}
)
EXPECT (
    {day: '2024-01-01', revenue: 100.0}
)
```

The `#<cte>` suffix selects one CTE within the referenced model. The CTE's upstream
chain — every CTE it depends on, directly and transitively — runs **as written**.
Only the model's external `smelt.<path>` dependencies are mockable via `PASSING`.

`PASSING` names in a CTE-level test are the model's external dependency paths (the
`smelt.<path>` refs reachable from the target CTE's dependency chain), not internal
CTE names. A `#<cte>` naming a CTE absent from the model is reported as `UnknownTestCte`.

!!! tip
    CTE-level tests are ideal for complex models with long CTE chains. Instead of mocking
    all upstream dependencies for the entire model, you can test each transformation step
    in isolation — treating each CTE as a function with defined inputs and outputs.

Here's an example testing a window function CTE:

```sql
smelt.test customer_quantiles_check AS (
    SELECT customer_id, revenue_decile, frequency_decile
    FROM smelt.int_customer_segments#customer_quantiles
)
PASSING customer_metrics AS (
    {customer_id: 1, customer_segment: 'Premium', order_count: 10, total_revenue: 1000.0, total_net_revenue: 900.0},
    {customer_id: 2, customer_segment: 'Standard', order_count: 5, total_revenue: 500.0, total_net_revenue: 450.0},
    {customer_id: 3, customer_segment: 'Basic', order_count: 2, total_revenue: 100.0, total_net_revenue: 90.0},
    {customer_id: 4, customer_segment: 'Premium', order_count: 8, total_revenue: 800.0, total_net_revenue: 720.0}
)
EXPECT (
    {customer_id: 1, revenue_decile: 1, frequency_decile: 1},
    {customer_id: 4, revenue_decile: 2, frequency_decile: 2},
    {customer_id: 2, revenue_decile: 3, frequency_decile: 3},
    {customer_id: 3, revenue_decile: 4, frequency_decile: 4}
)
```

### Property-based tests

When a `PASSING` row omits one or more columns that the CTE or model uses, smelt treats the
test as property-based. For each of the `cases` iterations (default 10):

1. smelt infers the type of each omitted column from the model's type checker.
2. Generates a random value of the appropriate type.
3. Executes the test with the augmented input data.
4. Checks that specified `EXPECT` columns match (unspecified output columns are ignored).
5. Verifies the query does not crash.

```sql
---
test:
  cases: 20
---
smelt.test daily_agg_property AS (
    SELECT day, revenue
    FROM smelt.daily_revenue#daily_agg
)
PASSING cleaned AS (
    -- user_id is omitted: random values are generated each iteration
    {amount: 100.0, created_at: '2024-01-01'},
    {amount: 200.0, created_at: '2024-01-01'}
)
EXPECT (
    -- only `revenue` is checked; other columns are ignored
    {revenue: 300.0}
)
```

If any iteration fails, the framework reports the random seed for reproduction.

### File placement

Each `smelt.test` declaration belongs in its own `.sql` file (or a file dedicated to
tests). Any `.sql` file that contains a `smelt.test` declaration is classified as a
**test file** by smelt — it will not be treated as a model, and other models cannot
reference it via `smelt.<name>`.

!!! note
    **Convention:** Place test files in a dedicated `tests/` directory and add it to `paths:`
    in `smelt.yml`. This keeps model files clean and makes it clear which files contain tests.

    ```yaml
    # smelt.yml
    paths:
      - models
      - tests
    ```

## Comparison behavior

### Set vs ordered comparison

By default, row order does not matter -- both actual and expected rows are compared as sets. Use `check_order: true` (in frontmatter) when row order is significant (e.g., testing window functions with specific ordering):

```sql
---
test:
  check_order: true
---
smelt.test check_rank AS (
    SELECT rank, user_id FROM smelt.revenue_report ORDER BY rank
)
PASSING revenue_report AS (
    {rank: 1, user_id: 42},
    {rank: 2, user_id: 17}
)
EXPECT (
    {rank: 1, user_id: 42},
    {rank: 2, user_id: 17}
)
```

### Column filtering

Only columns listed in `expect` are compared. Extra columns in the actual output are ignored. This lets you assert on the columns you care about without listing every column the model produces.

### Numeric tolerance

Floating-point values are compared with an epsilon of 1e-6. For example, an actual value of `300.0000001` matches an expected value of `300.0`.

### Type coercion

YAML values are automatically converted to SQL types:

| YAML value | SQL type | Example |
|------------|----------|---------|
| Integer | INTEGER | `42` |
| Float | DOUBLE | `3.14` |
| String | VARCHAR | `hello` or `'hello'` |
| Boolean | BOOLEAN | `true`, `false` |
| Null | NULL | `null` |
| Date string | DATE | `'2024-01-01'` (YYYY-MM-DD pattern) |
| Timestamp string | TIMESTAMP | `'2024-01-01 12:00:00'` (YYYY-MM-DD HH:MM:SS or T-separator) |

!!! note
    Strings matching the `YYYY-MM-DD` pattern are automatically cast to DATE, and strings matching `YYYY-MM-DD HH:MM:SS` (space or `T` separator) are cast to TIMESTAMP. There is no escape mechanism if you need a date-shaped string as VARCHAR.

## Running tests

### Run all tests

```bash
smelt test
```

### Filter by name

```bash
smelt test --select test_cohort_sizes
smelt test -s cohort -s user
```

!!! note
    The `--select` flag uses substring matching on test names. Passing `-s cohort` runs any test whose name contains "cohort". This differs from the graph-aware selector syntax used by `smelt run`.

### Show compiled SQL

```bash
smelt test --verbose
```

Use `--verbose` to see the SQL that smelt generates for each test. Helpful for debugging unexpected results.

### Show passing tests

```bash
smelt test --show-all
```

By default, only failing tests appear in the output. Use `--show-all` to also see passing tests.

## Output

```
smelt test

  PASS test_cohort_sizes (mart_cohort_retention::cohort_sizes)     0.02s
  FAIL test_user_activity (user_activity)                          0.03s

  1 passed, 1 failed, 2 total (0.05s)
```

For CTE tests, the output shows `(model::cte_name)`. For whole-model tests, it shows `(model_name)`.

The command exits with code 0 if all tests pass, or code 1 if any test fails.

!!! tip
    Use `smelt test` in CI to catch regressions. The non-zero exit code integrates naturally with CI systems.

## Further reading

- [SQL Models](sql-models.md) -- model syntax and YAML frontmatter
- [Materializations](materializations.md) -- all materialization types
- [CLI Commands](../reference/cli.md#smelt-test) -- full `smelt test` flag reference
