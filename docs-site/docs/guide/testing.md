# Testing

smelt lets you test your SQL models by defining expected inputs and outputs in test files, without needing a running database or executing your full pipeline.

## How it works

Tests are `.sql` files with `materialization: test` in YAML frontmatter. Each test specifies:

- A **model** to test (or a specific CTE within that model)
- **Mock input data** that replaces the model's dependencies
- **Expected output rows** to compare against

When you run `smelt test`, smelt compiles each test into a standalone SQL query with mock data substituted for dependencies, executes it against an in-memory DuckDB instance, and compares the actual output to your expected rows.

Tests live in a `tests/` directory (or co-located in model files) and must be discoverable via `paths:` in your `smelt.yml`:

```yaml
paths:
  - models
  - tests
```

## Test file format

Test files use the same YAML frontmatter as regular models:

```yaml
--- name: test_name ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily_agg
  inputs:
    cleaned_orders:
      - {order_id: 1, amount: 100.0, order_date: '2024-01-15'}
      - {order_id: 2, amount: 200.0, order_date: '2024-01-15'}
  expect:
    - {order_date: '2024-01-15', total_revenue: 300.0}
  check_order: false
  cases: 10
---
```

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `model` | Yes | | Name of the model to test |
| `target_cte` | No | | Test a specific CTE within the model instead of the full model |
| `inputs` | No | | Map of dependency names to arrays of row objects (mock data) |
| `expect` | Yes | | Array of expected output rows |
| `check_order` | No | `false` | If `true`, rows are compared positionally instead of as sets |
| `cases` | No | `10` | Number of iterations for property-based tests |

## Whole-model tests

A whole-model test runs the entire model with mocked dependencies. Each key in `inputs` corresponds to a `smelt.models....` call in the model's SQL.

```sql
--- name: test_user_activity ---
materialization: test
test:
  model: user_activity
  inputs:
    users:
      - {user_id: 1, user_name: Alice, signup_date: '2024-01-01'}
      - {user_id: 2, user_name: Bob, signup_date: '2024-02-01'}
    events:
      - {event_id: 1, user_id: 1, event_type: page_view, event_timestamp: '2024-01-15 10:00:00', properties: null}
      - {event_id: 2, user_id: 1, event_type: click, event_timestamp: '2024-01-16 11:00:00', properties: null}
      - {event_id: 3, user_id: 2, event_type: page_view, event_timestamp: '2024-02-15 09:00:00', properties: null}
  expect:
    - {user_id: 1, user_name: Alice, total_events: 2}
    - {user_id: 2, user_name: Bob, total_events: 1}
---
```

The framework replaces `smelt.models.users` and `smelt.models.events` with CTEs containing the mock data, then executes the rewritten query and compares the result to `expect`.

## CTE-level tests

CTE-level tests let you test a single CTE in isolation by setting `target_cte`. The `inputs` keys correspond to the CTEs that the target depends on directly -- not the model's external refs.

```sql
--- name: test_cohort_sizes ---
materialization: test
test:
  model: mart_cohort_retention
  target_cte: cohort_sizes
  inputs:
    cohort_base:
      - {customer_id: 1, cohort_date: '2024-01-01'}
      - {customer_id: 2, cohort_date: '2024-01-01'}
      - {customer_id: 3, cohort_date: '2024-02-01'}
  expect:
    - {cohort_date: '2024-01-01', cohort_size: 2}
    - {cohort_date: '2024-02-01', cohort_size: 1}
---
```

The framework extracts the `cohort_sizes` CTE from `mart_cohort_retention`, identifies that it depends on `cohort_base`, substitutes the mock data, and runs just the target CTE's SQL.

!!! tip
    CTE-level tests are ideal for complex models with long CTE chains. Instead of mocking all upstream dependencies for the entire model, you can test each transformation step in isolation -- treating each CTE as a function with defined inputs and outputs.

Here's another example testing a window function:

```sql
--- name: test_customer_quantiles ---
materialization: test
test:
  model: int_customer_segments
  target_cte: customer_quantiles
  inputs:
    customer_metrics:
      - {customer_id: 1, customer_segment: Premium, order_count: 10, total_revenue: 1000.0, total_net_revenue: 900.0}
      - {customer_id: 2, customer_segment: Standard, order_count: 5, total_revenue: 500.0, total_net_revenue: 450.0}
      - {customer_id: 3, customer_segment: Basic, order_count: 2, total_revenue: 100.0, total_net_revenue: 90.0}
      - {customer_id: 4, customer_segment: Premium, order_count: 8, total_revenue: 800.0, total_net_revenue: 720.0}
  expect:
    - {customer_id: 1, revenue_decile: 1, frequency_decile: 1}
    - {customer_id: 4, revenue_decile: 2, frequency_decile: 2}
    - {customer_id: 2, revenue_decile: 3, frequency_decile: 3}
    - {customer_id: 3, revenue_decile: 4, frequency_decile: 4}
---
```

## Property-based tests

When your input rows have fewer columns than the CTE or model expects, smelt treats the test as property-based. The framework:

1. Parses the target CTE/model to find all referenced columns
2. Uses type inference to determine the types of omitted columns
3. Generates random values for those columns
4. Runs the test multiple times (default 10, configurable via `cases`)
5. Each iteration: checks that specified output columns match and the query doesn't crash

```sql
--- name: test_daily_agg_property ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily
  cases: 20
  inputs:
    cleaned:
      # user_id omitted -- random values generated
      - {amount: 100.0, created_at: '2024-01-01'}
      - {amount: 200.0, created_at: '2024-01-01'}
  expect:
    # only revenue checked; other columns ignored
    - {revenue: 300.0}
---
```

This is useful when you want to assert that a transformation produces correct results regardless of what appears in columns it doesn't use. If any iteration fails, the framework reports the random seed for reproduction.

## Advanced tests (SQL body)

For complex mock data that's awkward to express in YAML, you can include SQL after the frontmatter closing `---`. The SQL body defines mock CTEs directly:

```sql
--- name: test_daily_agg_advanced ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily
  expect:
    - {day: '2024-01-01', revenue: 300.0}
---
WITH cleaned AS (
  SELECT i as user_id, (i * 50.0) as amount, '2024-01-01'::date as created_at
  FROM generate_series(1, 6) as t(i)
)
```

The SQL body's CTEs are used as mock data alongside any YAML `inputs`. This is useful for generating large datasets, sequences, or data that requires SQL expressions.

## Co-located tests

Tests can live as additional sections in the same file as the model they test. This works because smelt supports multi-section files -- each section separated by a `---` frontmatter block.

```sql
--- name: cleaned_orders ---
materialization: ephemeral
---
SELECT
    order_id,
    user_id,
    amount,
    created_at AS order_date
FROM smelt.models.raw_orders
WHERE status = 'completed'

--- name: test_cleaned_orders ---
materialization: test
test:
  model: cleaned_orders
  inputs:
    raw_orders:
      - {order_id: 1, user_id: 100, amount: 29.99, status: completed, created_at: '2024-01-15'}
      - {order_id: 2, user_id: 101, amount: 49.99, status: completed, created_at: '2024-01-15'}
      - {order_id: 3, user_id: 100, amount: 15.00, status: cancelled, created_at: '2024-01-16'}
  expect:
    - {order_id: 1, user_id: 100, amount: 29.99, order_date: '2024-01-15'}
    - {order_id: 2, user_id: 101, amount: 49.99, order_date: '2024-01-15'}
---
```

Tests reference models by name, not by file location, so co-located and separate tests behave identically.

!!! note
    **Convention:** Small projects often co-locate tests in model files to keep things together. Larger projects typically use a separate `tests/` directory to keep model files clean. Both approaches work -- choose what fits your team.

## Comparison behavior

### Set vs ordered comparison

By default, row order does not matter -- both actual and expected rows are compared as sets. Use `check_order: true` when row order is significant (e.g., testing window functions with specific ordering):

```yaml
test:
  model: my_model
  check_order: true
  inputs: ...
  expect:
    - {rank: 1, user_id: 42}
    - {rank: 2, user_id: 17}
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

!!! note
    Strings matching the `YYYY-MM-DD` pattern are automatically cast to DATE. If you need a string that looks like a date, this is a known limitation.

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
- [Materializations](materializations.md) -- all materialization types including `test`
- [CLI Commands](../reference/cli.md#smelt-test) -- full `smelt test` flag reference
