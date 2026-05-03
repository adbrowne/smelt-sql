---
feature: testing
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Testing

> **What this is.** A normative spec for the smelt testing framework — `materialization: test` files, mock data injection, CTE isolation, assertion semantics, and property-based test behavior.

## Surface

### Test file format

A test is a SQL file with `materialization: test` in YAML frontmatter. It declares a model to test, optional mock input data, and expected output rows:

```sql
--- name: test_daily_revenue ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily_agg           # optional: test a specific CTE
  inputs:
    orders:                       # dependency name → rows
      - {order_id: 1, amount: 100.0, order_date: '2024-01-15'}
      - {order_id: 2, amount: 200.0, order_date: '2024-01-15'}
  expect:
    - {order_date: '2024-01-15', total_revenue: 300.0}
  check_order: false              # default: false (set comparison)
  cases: 10                       # default: 10 (property-based iterations)
---
```

Tests are discovered by the same `model_paths` scan as SQL models. They can be co-located in multi-section files alongside the models they test, or placed in a separate `tests/` directory listed in `model_paths`.

### `test:` frontmatter key

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `model` | string | yes | — | Name of the model under test |
| `target_cte` | string | no | — | If set: test only this CTE in isolation. If absent: test the full model. |
| `inputs` | map | no | `{}` | Mock data: map from dependency name → list of row objects |
| `expect` | list | yes | — | Expected output rows |
| `check_order` | bool | no | `false` | If `true`: compare rows positionally. If `false`: compare as sets. |
| `cases` | integer | no | `10` | Number of property-based test iterations (used when inputs omit columns) |

### SQL body (advanced)

A test file may include SQL after the closing `---` of the frontmatter. The SQL body defines CTEs that are used as mock data alongside `inputs:`. This is useful for generating large or computed datasets that are awkward to express as YAML rows:

```sql
--- name: test_name ---
materialization: test
test:
  model: daily_revenue
  target_cte: daily
  expect:
    - {day: '2024-01-01', revenue: 300.0}
---
WITH cleaned AS (
  SELECT i AS user_id, (i * 50.0) AS amount, '2024-01-01'::DATE AS created_at
  FROM generate_series(1, 6) AS t(i)
)
```

### YAML value → SQL type coercion

| YAML value | SQL type |
|------------|----------|
| Integer (`42`) | `INTEGER` |
| Float (`3.14`) | `DOUBLE` |
| String `'YYYY-MM-DD'` pattern | `DATE` |
| Other string | `VARCHAR` |
| Boolean (`true` / `false`) | `BOOLEAN` |
| Null (`null`) | `NULL` |

Strings that match the `YYYY-MM-DD` pattern are automatically cast to `DATE`. If you need a string that looks like a date, this is a known limitation — no escape mechanism exists today.

### Comparison behavior

- **Columns**: Only columns listed in `expect` are compared. Extra columns in the actual output are ignored.
- **Floating point**: Values within `1e-6` of each other are treated as equal.
- **Row order**: When `check_order: false` (default), row order does not matter; both actual and expected are compared as multisets. When `check_order: true`, rows are compared positionally.

## Semantics

### Execution model

`smelt test` compiles each test into a standalone SQL query and executes it against a **fresh in-memory DuckDB instance**. No connection to the project's configured target database is made. The lifecycle per test:

1. Load the model under test (or the specified `target_cte`).
2. Substitute each dependency in `inputs` with a CTE containing the mock rows.
3. If a SQL body is present, include its CTEs as additional mock data.
4. Execute the rewritten query in in-memory DuckDB.
5. Compare actual output rows against `expect` per the comparison rules above.
6. Report PASS or FAIL.

### Whole-model tests

When `target_cte` is absent, the entire model SQL is compiled with mock data substituted for all `smelt.models.<name>` and `smelt.sources.<name>` references named in `inputs`. Dependencies not listed in `inputs` are replaced with empty CTEs (zero rows).

### CTE-level tests

When `target_cte` is set, smelt:
1. Extracts the named CTE from the model's WITH clause.
2. Identifies which upstream CTEs that CTE depends on directly.
3. Substitutes those upstream CTEs with mock data from `inputs`.
4. Executes only the target CTE's SQL expression.

`inputs` keys in a CTE-level test must match the **CTE names** that the target CTE depends on, not the model's external refs.

### Property-based tests

A test is treated as property-based when one or more columns of an input row are **omitted from the YAML**. For each of the `cases` iterations:
1. smelt infers the type of each omitted column from the model's type checker.
2. Generates a random value of the appropriate type.
3. Executes the test with the augmented input data.
4. Checks that specified output columns in `expect` match (unspecified output columns are ignored).
5. Verifies the query does not crash.

Each iteration uses a different random seed derived from the test's global seed. If any iteration fails, the failure report includes the random seed that caused it for reproduction.

### Tests always use DuckDB

`smelt test` always runs against in-memory DuckDB, regardless of the project's configured targets. Tests on Spark-only projects are not validated against Spark semantics. This is a known design gap.

## Design

**Test-as-materialization.** Using `materialization: test` rather than a separate test file format means the parser, type checker, LSP, and model discovery system all handle test files uniformly. Tests are discovered by `model_paths`, not a separate `test_paths`. The tradeoff is that test models appear in `smelt explain` output and must be explicitly excluded from execution runs (they are never materialized by `smelt run`).

**Mock by dependency name.** Input mock data is keyed by the dependency's name (the model or CTE name), not by file path or address. This is consistent with `smelt.models.<name>` addressing and lets tests read naturally.

**Set comparison by default.** `check_order: false` is the safe default. Most models do not produce ordered output, and ordering in SQL is non-deterministic unless an `ORDER BY` is present. Requiring `check_order: true` explicitly for ordered output avoids brittle tests that depend on DuckDB's internal sort order.

**CTE isolation.** Isolating individual CTEs as testable units was a primary motivation for the framework — it allows complex models with long CTE chains to be tested incrementally, one transformation at a time, without mocking the entire upstream graph.

**SQL body as escape hatch.** YAML row data is practical for small fixtures but awkward for generated sequences, large datasets, or computed values. The SQL body CTE mechanism provides an escape without requiring a new syntax.

## Constraints & Invariants

1. **Tests run in-memory on DuckDB.** No connection to the project's configured target is made during `smelt test`.
2. **Test models are never materialized by `smelt run` or `smelt build`.** `materialization: test` models are excluded from execution runs. They cannot have `incremental` config or `target` overrides.
3. **`expect` is required.** A test with no `expect` rows is invalid.
4. **`inputs` keys are dependency names.** For whole-model tests: names of `smelt.models.<name>` or `smelt.sources.<name>` references. For CTE tests: names of upstream CTEs the target CTE depends on.
5. **Column comparison uses only `expect` columns.** Extra actual columns are never treated as failure.

## Known Divergences / Open Questions

- **Unlisted input dependencies replaced with empty CTEs.** Dependencies not listed in `inputs` receive zero rows. This is intentional but not clearly documented — a query that JOINs an unlisted dependency will silently get no rows from it.
- **Date string auto-cast is opt-out impossible.** Strings matching `YYYY-MM-DD` are always cast to `DATE`. There is no way to pass a date-shaped string as `VARCHAR` in `inputs`.
- **Property-based test column discovery.** The mechanism for determining which columns are "omitted" (triggering property-based generation) and how their types are inferred is not fully specified. Behavior when type inference is unavailable is undefined.
- **`cases: 0` behavior.** Setting `cases: 0` when inputs have omitted columns may result in no iterations. Whether this is PASS or an error is undefined.
- **Spark test gap.** Tests always run on DuckDB. Spark-specific function behavior (MERGE semantics, Parquet type handling) cannot be tested with `smelt test`.

## References

- **Code**:
  - `crates/smelt-core/src/metadata.rs` — `TestConfig`, `ColumnTest`
  - `crates/smelt-core/src/discovery.rs` — `ModelFile::is_test()`, `ModelFile::test_config()`
  - `crates/smelt-cli/src/commands/` — `smelt test` command implementation
- **User docs**:
  - `docs-site/docs/guide/testing.md`
- **Related specs**:
  - `models.md` — `materialization: test` and the `test:` frontmatter key
  - `seeds.md` — seeds as mock data sources in tests
  - `cli.md` — `smelt test` command behavior, exit codes
