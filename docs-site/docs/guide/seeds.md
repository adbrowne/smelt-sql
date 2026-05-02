# Seeds

Seeds are CSV files that smelt loads into your database as tables. They are useful for small reference datasets, lookup tables, and test data that you want to version-control alongside your models.

## Directory structure

Place CSV files in the `seeds/` directory (configurable via `seed_paths` in `smelt.yml`). Subdirectories map to schema names in the database.

```
my_project/
  seeds/
    raw_orders.csv
    raw/
      users.csv
      transactions.csv
      events.csv
      sessions.csv
  models/
    ...
  smelt.yml
```

The path of a CSV under `seed_paths` determines both the qualified table name written to the database and the reference you use in models:

| Filesystem location | Loaded as | Reference in models |
|---|---|---|
| `seeds/raw_orders.csv` | `<target_schema>.raw_orders` | `smelt.models.raw_orders` |
| `seeds/raw/users.csv` | `raw.users` | `smelt.sources.raw.users` |
| `seeds/raw/events.csv` | `raw.events` | `smelt.sources.raw.events` |

Top-level seeds land in the active target's `schema:` (default `main`) and are addressed as `smelt.models.<name>` — the same call surface as a SQL model. Subdirectory seeds become their own schema and are addressed as `smelt.sources.<schema>.<name>`. The architectural target is to address every seed uniformly under `smelt.seeds.<path>`; this is documented in [`docs/specs/seeds.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/specs/seeds.md) and is being migrated to.

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

## Column type inference

Two type-inference passes operate on every seed CSV. They agree on the shapes both can recognise, and disagree on shapes only one can.

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

smelt's own inferencer (used for diagnostics, completions, and `smelt table`) samples the first 10 data rows and recognises `BOOLEAN`, `INTEGER`, `DOUBLE`, `DATE` (`YYYY-MM-DD`-shaped values), `TIMESTAMP` (`YYYY-MM-DD HH:MM:SS`-shaped, optional fractional seconds), and `Text`. Anything it cannot classify falls back to `Text`.

The compile-time and runtime inferencers agree on the recognised types — `BOOLEAN`, `DATE`, `TIMESTAMP`, `INTEGER`, `DOUBLE`. The two pass-disagreements you can hit in practice:

- **`TIMESTAMP WITH TIME ZONE` columns.** The compile-time inferencer never emits the with-zone variant; a column whose values include `2025-01-01 12:00:00+00` falls back to `Text` even when DuckDB would store it as `TIMESTAMPTZ`.
- **`DECIMAL`-shaped columns.** DuckDB sometimes types a bounded numeric as `DECIMAL(p,s)`; the compile-time inferencer always emits `Double` for any parseable-as-`f64` column.

When the two disagree, the compile-time inferencer is the one your editor uses for type-checking downstream models. The conservative fix is to cast explicitly in the first staging model that consumes the seed:

```sql
SELECT
  CAST(amount AS DECIMAL(10, 2)) AS amount,
  CAST(event_ts AS TIMESTAMP WITH TIME ZONE) AS event_ts,
  ...
FROM smelt.models.raw_orders
```

To inspect what smelt's type-checker thinks the columns are (the canonical compile-time view):

```bash
smelt table raw_orders
```

To see what DuckDB actually inferred at runtime after `smelt seed` or `smelt build`:

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

A top-level seed file `seeds/raw_orders.csv` is referenced as `smelt.models.raw_orders` — the same call surface as a SQL model:

```sql
-- models/orders_summary.sql
SELECT
  order_date,
  COUNT(*) AS order_count,
  SUM(amount) AS total_amount
FROM smelt.models.raw_orders
GROUP BY 1
```

A subdirectory seed file `seeds/raw/users.csv` is referenced as `smelt.sources.raw.users`:

```sql
SELECT
  p.product_id,
  p.product_name,
  ch.category_name
FROM smelt.models.products AS p
JOIN smelt.models.category_hierarchy AS ch
  ON p.category_code = ch.category_code
```

smelt resolves column types from the CSV headers and data, so you get full type inference and LSP diagnostics for seed columns either way.

!!! note
    Top-level seeds (files directly in the seed directory) are available as `smelt.models.<name>`. Subdirectory seeds are loaded into the schema named after the parent directory and accessed via `smelt.sources.<schema>.<name>`.

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
