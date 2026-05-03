# Seeds

Seeds are CSV files that smelt loads into your database as tables. They are useful for small reference datasets, lookup tables, and test data that you want to version-control alongside your models.

## Directory structure

Place CSV files anywhere under the directories listed in `paths:` in `smelt.yml`. Seeds are CSV files; they can live alongside SQL models in the same directories.

```
my_project/
  models/
    orders_summary.sql
    raw_orders.csv
    raw/
      users.csv
      transactions.csv
  smelt.yml
```

Or keep seeds in a dedicated directory by adding it to `paths:`:

```
my_project/
  models/
    orders_summary.sql
  seeds/
    raw_orders.csv
    raw/
      users.csv
  smelt.yml   # paths: [models, seeds]
```

The address is the path from the scan root to the file stem, dot-separated. The DB name joins the address segments with `_`:

| Filesystem location | Address | DB name (`main` schema) |
|---|---|---|
| `seeds/raw_orders.csv` | `smelt.raw_orders` | `main.raw_orders` |
| `seeds/raw/users.csv` | `smelt.raw.users` | `main.raw_users` |

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

smelt parses the CSV, infers column types from the data, and loads the result via Arrow into the target backend. The table is created (or replaced) each time you run the seed command.

## Column type inference

smelt owns the type inferencer. There is one code path; compile time (LSP, `smelt table`) samples the first 100 rows, and runtime (`smelt seed`, `smelt build`) reads the whole file. The two phases cannot disagree by construction.

### Type precedence

Types are inferred in priority order:

| Column shape | Inferred type |
|---|---|
| `true` / `false` (case-insensitive) | `BOOLEAN` |
| `2025-01-01` (`YYYY-MM-DD`, year 1000–9999) | `DATE` |
| `2025-01-01 12:00:00` (space separator, optional fractional seconds) | `TIMESTAMP` |
| `1`, `42`, `-7` (fits in i64) | `INTEGER` |
| `3.14`, `-0.5` (decimal literal, `p ≤ 18`, `s ≤ 4`) | `DECIMAL(p, s)` |
| `1.5e10`, large decimals | `DOUBLE` |
| Anything else | `VARCHAR` |

Empty cells are always `NULL`, regardless of the column type.

### What falls back to VARCHAR

- ISO-8601 timestamps with a `T` separator: `2025-01-10T08:00:00` → `VARCHAR`
- Timestamps with a timezone suffix (`Z`, `+00`, `-05:00`): `2025-01-10 08:00:00Z` → `VARCHAR`
- Decimal values with more than 4 fractional digits: `3.14159` → `DOUBLE` (not `DECIMAL`)
- Decimal values with precision > 18: falls through to `DOUBLE`
- Any other value that cannot be parsed as one of the above types

If you need a specific type (for example `TIMESTAMP WITH TIME ZONE`), cast explicitly in the first staging model:

```sql
SELECT
  CAST(amount AS DECIMAL(10, 2)) AS amount,
  CAST(event_ts AS TIMESTAMPTZ) AS event_ts,
  ...
FROM smelt.raw_orders
```

To inspect what smelt infers for a seed's columns:

```bash
smelt table raw_orders
```

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

Seeds are CSV files discovered under the unified `paths:` list in `smelt.yml`. The default is `paths: [models]`, but you can add any directory:

```yaml
paths:
  - models
  - seeds
  - test_data
```

Every directory is scanned recursively. CSV files are classified as seeds; `.sql` files as models or functions. Subdirectory structure within a path produces address segments that become part of the DB-name mapping (see [Referencing seeds in models](#referencing-seeds-in-models)).

## Target selection

Like model runs, seed loading respects the `--target` flag:

```bash
smelt seed --target dev
smelt seed --target spark
```

## Referencing seeds in models

Seeds are addressed by their path relative to the scan root. A seed at `seeds/raw_orders.csv` (under `paths: [seeds]`) is addressed as `smelt.raw_orders`; a seed at `seeds/raw/users.csv` is `smelt.raw.users`.

The default DB name maps address segments to `<target_schema>.<segments_joined_by_>`:

| Filesystem location (under `paths: [seeds]`) | Address | DB name |
|---|---|---|
| `seeds/raw_orders.csv` | `smelt.raw_orders` | `main.raw_orders` |
| `seeds/raw/users.csv` | `smelt.raw.users` | `main.raw_users` |
| `seeds/lookup/regions.csv` | `smelt.lookup.regions` | `main.lookup_regions` |

Reference seeds in models with `smelt.<address>`:

```sql
-- models/orders_summary.sql
SELECT
  order_date,
  COUNT(*) AS order_count,
  SUM(amount) AS total_amount
FROM smelt.raw_orders
GROUP BY 1
```

smelt resolves column types from the CSV headers and data, so you get full type inference and LSP diagnostics for seed columns.

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
