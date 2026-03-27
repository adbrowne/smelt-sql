# SQL Unit Testing Design

## Problem

Unit testing SQL transformations is an unsolved problem. Existing tools (dbt, SQLMesh) test whole models or peek at CTE outputs by running the full query. Nobody lets you test a CTE in isolation by mocking its dependencies — treating it as a function with parameters.

The challenge compounds with large models: a model with staging, joins, aggregation, and window functions produces a complex query where bugs hide in intermediate steps. Testing the whole output doesn't localize failures.

## Prior Art

| Tool | Approach | Limitation |
|------|----------|------------|
| **dbt** (v1.8+) | YAML fixtures for whole models | No CTE testing. Workaround: break into ephemeral models (awkward). |
| **SQLMesh** | Runs full query, validates CTE outputs | Still executes entire query. Top-level CTEs only. No macro support. Weak error reporting. |
| **tSQLt** | Fake tables/stored procs (SQL Server) | Database-specific. Tests procedures, not transformations. |
| **dbt-unit-testing** | CTE substitution via Jinja | Bolt-on. Fragile macro-based approach. |

**Key gap**: No tool treats CTEs as independently testable units with mockable dependencies.

## Design

### Core Idea: Tests as Models

Tests are model sections with `materialization: test`. They participate in the existing infrastructure — discovery, multi-model files, parsing, LSP (diagnostics, goto-definition, hover).

Two test modes:
1. **CTE test**: Tests a single CTE in isolation, mocking ALL its direct dependencies
2. **Whole-model test**: Tests an entire model, mocking its `smelt.ref()` inputs

Three specification levels:
1. **Exact test**: All input/output columns specified — deterministic equality check
2. **Partial test → property-based**: Only some columns specified — framework generates random values for omitted columns, asserts specified output columns match and query doesn't crash
3. **Advanced test**: SQL body generates complex mock data

### Test Syntax

#### Exact CTE Test (most common)

```yaml
--- name: test_daily_agg ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily
  inputs:
    cleaned:
      - {user_id: 1, amount: 100.0, created_at: '2024-01-01'}
      - {user_id: 2, amount: 200.0, created_at: '2024-01-01'}
  expect:
    - {day: '2024-01-01', revenue: 300.0}
---
```

The framework:
1. Extracts the `daily` CTE's SQL from `daily_revenue`
2. Identifies `daily`'s dependency: `cleaned`
3. Substitutes `cleaned` with mock data from `inputs`
4. Executes: `WITH cleaned AS (mock data) <daily's SQL body>`
5. Compares result to `expect`

#### Exact Whole-Model Test

```yaml
--- name: test_daily_revenue ---
materialization: test
test:
  model: daily_revenue
  inputs:
    raw_orders:
      - {order_id: 1, amount: 100.0, status: completed, created_at: '2024-01-01'}
  expect:
    - {day: '2024-01-01', revenue: 100.0}
---
```

The framework:
1. Takes `daily_revenue`'s full SQL
2. Replaces `smelt.ref('raw_orders')` with a CTE containing mock data
3. Executes the rewritten query
4. Compares result to `expect`

#### Property-Based Test (partial columns)

```yaml
--- name: test_daily_agg_property ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily
  cases: 20  # optional, default 10
  inputs:
    cleaned:
      # user_id omitted → random values generated
      - {amount: 100.0, created_at: '2024-01-01'}
      - {amount: 200.0, created_at: '2024-01-01'}
  expect:
    # only revenue checked; other columns ignored
    - {revenue: 300.0}
---
```

The framework:
1. Parses the `daily` CTE to determine what columns it reads from `cleaned`
2. Uses type inference to determine the types of omitted columns (e.g., `user_id: INTEGER`)
3. Generates random values for omitted input columns
4. Runs the test N times (default 10, configurable via `cases:`)
5. Each run: asserts specified output columns match expected values AND query doesn't crash
6. If any run fails: reports the failing random seed and generated values for reproduction

**Schema inference**: The CTE's SQL body references columns from its dependencies. The type inference system (already implemented in `smelt-db`) can determine the types of those columns. No need to resolve the full upstream model schema — just analyze what the CTE reads.

#### Advanced Test (SQL body)

```yaml
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

SQL body provides mock CTEs directly. Useful for complex data generation that's awkward in YAML.

### Test Location

Tests can live in either location:

**Co-located** — additional sections in the same model file:
```sql
--- name: daily_revenue ---
materialization: table
---
WITH cleaned AS (
  SELECT user_id, amount, created_at FROM smelt.ref('raw_orders') WHERE status = 'completed'
),
daily AS (
  SELECT DATE(created_at) as day, SUM(amount) as revenue FROM cleaned GROUP BY 1
)
SELECT * FROM daily

--- name: test_daily_agg ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily
  inputs:
    cleaned:
      - {user_id: 1, amount: 100.0, created_at: '2024-01-01'}
  expect:
    - {day: '2024-01-01', revenue: 300.0}
---
```

**Separate** — in a `tests/` directory or `*.test.sql` files, referencing models by name.

Both work because tests reference models by name, not by file location.

### Execution Model

- **Always DuckDB**: Tests execute against an in-memory DuckDB instance. Millisecond-fast, no external dependencies.
- **Trust dialect translation**: smelt's backend translation layer ensures SQL semantics are preserved across backends. If it works on DuckDB, it works on Spark/Postgres.
- **Row comparison**:
  - Default: set comparison (order doesn't matter). Both sides sorted by all columns before comparing.
  - If the test SQL includes ORDER BY: ordered row-by-row comparison.
- **Isolation**: Each test gets a fresh in-memory DuckDB connection. No shared state.

### CLI

```
smelt test                        # run all tests
smelt test --select test_name     # run specific test(s)
smelt test --verbose              # show compiled SQL for each test
smelt test --show-all             # show passing tests too (default: only failures)
```

Output:
```
smelt test

  PASS test_daily_agg (daily_revenue::daily)                0.02s
  FAIL test_weekly_agg (weekly_revenue::weekly)             0.03s
  PASS test_daily_revenue (daily_revenue)                   0.01s
  PASS test_daily_agg_property (daily_revenue::daily) [20]  0.15s

  3 passed, 1 failed, 4 total (0.21s)
```

Property tests show the case count in brackets.

### Error Reporting

```
FAIL test_weekly_agg (model: weekly_revenue, cte: weekly)

  Expected 1 row, got 2 rows.

  Missing rows (expected but not found):
    {week: '2024-W01', revenue: 300.0}

  Unexpected rows (found but not expected):
    {week: '2024-W01', revenue: 200.0}
    {week: '2024-W02', revenue: 100.0}

  Compiled SQL:
    WITH cleaned AS (
      SELECT * FROM (VALUES
        (1, 100.0, '2024-01-01'::DATE),
        (2, 200.0, '2024-01-03'::DATE)
      ) AS t(user_id, amount, created_at)
    )
    SELECT DATE_TRUNC('week', created_at) as week, SUM(amount) as revenue
    FROM cleaned GROUP BY 1
```

For property-based test failures:
```
FAIL test_daily_agg_property (model: daily_revenue, cte: daily) [case 7/20]

  Failing seed: 0xABCD1234 (reproduce with: smelt test --seed 0xABCD1234 --select test_daily_agg_property)

  Generated inputs for 'cleaned':
    {user_id: -2147483648, amount: 100.0, created_at: '2024-01-01'}
    ...

  Expected: {revenue: 300.0}
  Actual:   query crashed with: "integer overflow in SUM()"
```

### LSP Integration

Tests get LSP support for free because they are models:
- **Diagnostics**: Parse errors in test SQL body, undefined references
- **Goto-definition**: `smelt.ref()` in test SQL body navigates to the referenced model
- **Hover**: Type information on expressions in test SQL body

Future LSP enhancements (not in initial implementation):
- Validate `test.model` references an existing model
- Validate `test.target_cte` references a CTE that exists in the model
- Validate `test.inputs` keys match CTE dependency names or ref names
- "Run test" code lens above test sections

## Implementation

### Phase 1: Metadata & Discovery

**`smelt-core/src/config.rs`**: Add `Test` to `Materialization` enum.

**`smelt-core/src/metadata.rs`**: Add `TestConfig` struct with fields: `model`, `target_cte`, `inputs`, `expect`, `cases`. Add `test: Option<TestConfig>` to `ModelMetadata`.

**`smelt-core/src/discovery.rs`**: Add `is_test()` and `test_config()` helpers to `ModelFile`.

### Phase 2: CTE Extraction

**New: `smelt-cli/src/test_compiler.rs`**

`extract_ctes(sql) -> Vec<CteInfo>`:
- Parse SQL with `smelt_parser::parse()`
- Walk `WithClause::ctes()` for each CTE's name and body
- Detect dependencies: parse each CTE body, find table references matching other CTE names

`CteInfo { name, body, dependencies }` — the building block for test compilation.

### Phase 3: YAML-to-SQL & Test Compilation

**`smelt-cli/src/test_compiler.rs`**

`yaml_rows_to_sql(name, rows) -> String`:
- Column ordering: use `IndexMap` or derive from first row
- Type mapping: YAML integers → SQL integers, floats → floats, strings → VARCHAR (with date heuristic for `YYYY-MM-DD`), bools → BOOLEAN, null → NULL
- Output: `SELECT * FROM (VALUES (...), (...)) AS t(col1, col2)`

`compile_cte_test(model_sql, target_cte, inputs, sql_body) -> String`:
- Extract target CTE and its dependencies
- Substitute dependencies with mock CTEs (from YAML or SQL body)
- Assemble executable SQL

`compile_whole_model_test(model_sql, inputs, sql_body) -> String`:
- Replace `smelt.ref()` calls with mock CTEs
- Reuse existing ref resolution and ephemeral CTE prepending from `compiler.rs`

### Phase 4: Property-Based Test Generation

**`smelt-cli/src/test_compiler.rs`**

When input rows have fewer columns than the CTE expects:
1. Parse the target CTE to find all column references to the mocked dependency
2. Use type inference to determine types of missing columns
3. Generate random values per type (integers, floats, strings, dates, booleans, nulls)
4. Run N iterations (default 10, configurable via `cases:` in frontmatter)
5. Each iteration: fill in random values, compile, execute, check specified output columns

Random generation strategy:
- Use a seeded RNG (report seed on failure for reproduction)
- Include edge cases: NULL, empty string, MIN/MAX integers, NaN for floats, epoch dates
- `--seed` flag for reproducing specific failures

When expected output has fewer columns than actual:
- Only compare specified columns, ignore the rest

### Phase 5: Test Execution & Comparison

**New: `smelt-cli/src/test_runner.rs`**

Execution: `duckdb::Connection::open_in_memory()` per test. Execute compiled SQL, collect Arrow RecordBatches.

Comparison:
- Convert Arrow rows and YAML rows to a common `ComparableRow` type
- Handle type coercion: Arrow Int32/64 ↔ YAML integer, Float64 ↔ YAML float (epsilon), Utf8 ↔ YAML string, Date32 ↔ YAML date string
- Set comparison: sort both sides, diff
- Ordered comparison: row-by-row when ORDER BY present

Diff generation: missing rows, unexpected rows, column-level mismatches.

### Phase 6: CLI Command

**`smelt-cli/src/main.rs`**: Add `Test` variant to `Commands` enum. Implement handler that discovers tests, compiles, executes, reports.

Exclude test models from `smelt run`, `build`, `explain` commands.

### Phase 7: Integration & Polish

- Wire up modules in `smelt-cli/src/lib.rs`
- Add test models to `examples/ephemeral_demo/` for dogfooding
- Ensure LSP handles test models gracefully

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Test format | Tests-as-models | Reuses existing infrastructure. Gets LSP support for free. |
| Execution engine | Always DuckDB | Fast, bundled, no deps. Trust dialect translation for other backends. |
| CTE isolation | Mock ALL direct dependencies | Simpler mental model. Each test is self-contained. |
| Partial columns | Property-based testing | Novel capability. Leverages existing type inference and proptest-style infrastructure. |
| Row comparison | Set by default, ordered with ORDER BY | SQL results are unordered by default. Explicit ORDER BY signals intent. |
| Default PBT cases | 10 | Fast feedback. Configurable via `cases:` for thoroughness. |

## Open Questions

1. **Recursive CTEs**: Should tests support targeting recursive CTEs? Probably defer — they're rare and complex.
2. **Multi-CTE chains**: Currently scoped to single CTE isolation. Could later add a mode that tests a chain of CTEs together (mock only the earliest dependencies).
3. **Snapshot testing**: Could we support "golden file" mode where expected output is auto-captured on first run and compared on subsequent runs? Nice-to-have, not essential for v1.
4. **Test-level configuration**: Beyond `cases:`, are there other per-test knobs needed? (timeout, specific DuckDB settings, etc.)
5. **YAML date ambiguity**: Unquoted `2024-01-01` in YAML may parse as a date or string depending on the parser. Need to test `serde_yaml` behavior and document requirements.

## What Makes This Novel

1. **CTE-as-function**: No existing tool treats CTEs as independently testable functions with mockable dependencies.
2. **Partial specs → property tests**: Omitting columns automatically creates property-based tests. This is unheard of in SQL testing.
3. **Tests are models**: Full LSP support, same discovery/parsing infrastructure, co-locatable with code.
4. **Compiler-powered**: smelt's parser and type system enable CTE extraction and schema inference that string-manipulation approaches can't match.
5. **Zero-config execution**: Bundled DuckDB means `smelt test` works immediately with no setup.
