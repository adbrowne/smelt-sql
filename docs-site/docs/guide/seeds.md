# Seeds

Seeds are CSV files that smelt loads into your database as tables. They are useful for small reference datasets, lookup tables, and test data that you want to version-control alongside your models.

## Directory structure

Place CSV files in the `seeds/` directory (configurable via `seed_paths` in `smelt.yml`). Subdirectories map to schema names in the database.

```
my_project/
  seeds/
    raw/
      users.csv
      transactions.csv
      events.csv
      sessions.csv
  models/
    ...
  smelt.yml
```

In this example, `seeds/raw/users.csv` is loaded as the table `raw.users`.

## CSV format

Seeds are standard CSV files with a header row:

```csv
user_id,user_name,signup_date
1,Alice,2025-01-01
2,Bob,2025-01-02
3,Charlie,2025-01-03
4,Diana,2025-01-04
5,Eve,2025-01-05
```

smelt infers column types from the data. The table is created (or replaced) each time you run the seed command.

## Type inference

Two type-inference passes operate on every seed CSV — and they don't always agree.

### At runtime (DuckDB)

Seeds are loaded with DuckDB's `read_csv_auto()`. DuckDB samples the file and assigns a type per column:

| Column shape | DuckDB type |
|---|---|
| `1`, `42`, `-7` | `INTEGER` (or `BIGINT` if values exceed INT range) |
| `1.5`, `100.00` | `DOUBLE` |
| `true` / `false` (case-insensitive) | `BOOLEAN` |
| `2025-01-01` | `DATE` |
| `2025-01-01 12:00:00` | `TIMESTAMP` |
| Anything else | `VARCHAR` |

`DECIMAL` is **not** auto-inferred — numeric columns with decimal points always become `DOUBLE`. If you need `DECIMAL` precision, cast in a staging model (`CAST(amount AS DECIMAL(10, 2))`).

### At compile time (smelt LSP / `smelt table`)

smelt's own inferencer (used for diagnostics, completions, and `smelt table`) is simpler and only recognises four types: `BOOLEAN`, `INTEGER`, `DOUBLE`, and `TEXT`. It samples the first 10 rows. Date- and timestamp-shaped columns that DuckDB recognises as `DATE`/`TIMESTAMP` show up as `TEXT` in `smelt table` output.

This divergence means a seed column may **load as `DATE`** at runtime but **show as `TEXT`** in your editor. The conservative fix is to cast explicitly in the first staging model that consumes the seed:

```sql
SELECT
  CAST(order_date AS DATE) AS order_date,
  CAST(amount AS DECIMAL(10, 2)) AS amount,
  ...
FROM smelt.models.raw_orders
```

To see what DuckDB actually inferred for a seed after `smelt seed` or `smelt build`:

```bash
duckdb my-project.duckdb -c 'DESCRIBE raw_orders'
```

There is currently no `seeds.yml` or per-seed frontmatter for overriding types — explicit casts in downstream models are the supported escape hatch.

## Commands

### Load all seeds

```bash
smelt seed
```

### Load and display results

```bash
smelt seed --show-results
```

This prints the loaded data in a table format after seeding, which is helpful for verifying that the CSV was parsed correctly.

### Load specific seeds

```bash
smelt seed --select users
smelt seed --select raw.users
```

Use `--select` to load only specific seed files by name or by `schema.name`.

### Build (seed + run)

```bash
smelt build
```

The `build` command combines seeding and model execution in one step. It loads all seeds first, then runs all models. This is the most common command during development.

!!! tip
    Use `smelt build` when starting fresh or resetting your development database. It ensures seeds are loaded before any models that depend on them run.

## Configuration

The seed directory defaults to `seeds/`. Override it in `smelt.yml`:

```yaml
seed_paths:
  - seeds
  - test_data
```

Multiple directories are supported. Each is scanned for CSV files, and subdirectories map to schema names as described above.

## Target selection

Like model runs, seed loading respects the `--target` flag:

```bash
smelt seed --target dev
smelt seed --target spark
```

## Referencing seeds in models

Seeds in the top-level seed directory (not in subdirectories) are available as `smelt.models.<name>` targets, just like SQL models:

```sql
SELECT
  p.product_id,
  p.product_name,
  ch.category_name
FROM smelt.models.products AS p
JOIN smelt.models.category_hierarchy AS ch
  ON p.category_code = ch.category_code
```

Here `category_hierarchy` is a seed CSV (`seeds/category_hierarchy.csv`). smelt resolves its column types from the CSV headers and data, so you get full type inference and LSP diagnostics for seed columns.

!!! note
    Only top-level seeds (files directly in the seed directory, not in subdirectories) are available as `smelt.models.<name>` targets. Subdirectory seeds are loaded into their respective schemas and accessed via `smelt.sources.<name>`.

## When to use seeds

Seeds work well for:

- **Reference data** -- Country codes, status enums, category mappings
- **Test data** -- Small datasets for development and testing
- **Static lookups** -- Data that rarely changes and is small enough to version in git

!!! warning
    Seeds are not designed for large datasets. CSV files are fully loaded into memory and inserted as a single batch. For datasets larger than a few thousand rows, use [Sources](sources.md) instead and load the data with your ingestion pipeline.

## Further reading

- [Data Generation](datagen.md) for generating large deterministic datasets with configurable distributions
- [Sources](sources.md) for referencing external tables not managed by smelt
- [Targets and Backends](targets.md) for configuring where seeds are loaded
