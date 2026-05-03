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

If you need a specific type, cast explicitly in the first staging model:

```sql
SELECT
  CAST(amount AS DOUBLE) AS amount,        -- inferred DECIMAL(p,s) → DOUBLE
  CAST(order_id AS INTEGER) AS order_id,   -- inferred INTEGER, but explicit is safer
  CAST(event_ts AS TIMESTAMPTZ) AS event_ts,
  ...
FROM smelt.raw_orders
```

Common cases where the inferred type needs a cast:

- **Money / price columns** (`29.99`, `100.00`) — inferred as `DECIMAL(p, s)`. Downstream `SUM` or `COALESCE` may return `DECIMAL(38, 2)` rather than `DOUBLE`. Cast to `DOUBLE` in staging if the spec requires it, or use `CAST(COALESCE(SUM(col), 0.0) AS DOUBLE)` in the mart.
- **ISO-8601 timestamps** (`2025-01-10T08:00:00`) — inferred as `VARCHAR` (the `T` separator is not recognized). Cast to `TIMESTAMP` or `TIMESTAMPTZ` in staging.
- **IDs stored as integers** — inferred as `INTEGER` when values fit in i64, which is usually correct. If joining to a column smelt infers as `BIGINT`, an explicit `CAST(id AS INTEGER)` removes the ambiguity.

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

Seeds and SQL models share the same flat namespace — there is no `smelt.models.*` prefix. A seed at `seeds/raw_orders.csv` and a model at `models/stg_orders.sql` are both referenced as `smelt.raw_orders` and `smelt.stg_orders` respectively.

### Joining seeds together

A common staging pattern joins two seeds to enrich a fact table with dimension attributes:

```sql
-- models/stg_orders.sql
---
name: stg_orders
materialization: table
---
SELECT
  o.order_id,
  o.order_date,
  o.amount,
  c.name AS customer_name,
  c.country
FROM smelt.raw_orders o
LEFT JOIN smelt.raw_customers c ON o.customer_id = c.customer_id
```

Column names in the output must match what the spec requires. Seed column names are locked to CSV headers — alias them in the staging model if the spec uses different names (e.g. CSV has `name`, spec requires `customer_name`).

smelt resolves column types from the CSV headers and data, so you get full type inference and LSP diagnostics for seed columns.

## LSP affordances

The smelt language server provides editor support for seed files:

### Missing sidecar warning

When you add a CSV file to your project without a sibling `.yml` sidecar, the language server emits a workspace warning:

```
Seed schema is inferred and may drift if the CSV changes — pin it
```

This warning appears at the top of the CSV file in your editor's Problems panel. Inferred schemas are computed from the first 100 rows at compile time; if the CSV later gains new columns or the data changes type, the LSP's understanding can silently drift. Pinning the schema with a sidecar eliminates that risk.

The warning disappears as soon as a sibling `.yml` file exists.

### "Pin schema to sidecar YAML" code action

When a CSV file has no sidecar, the language server offers a quick-fix code action: **"Pin schema to sidecar YAML"**.

Applying the action:
1. Reads the entire CSV file (all rows, not just the 100-row compile-time sample)
2. Runs the type inferencer over every row
3. Writes a sibling `<name>.yml` next to the CSV with `columns:` and `type:` entries

The resulting file looks like:

```yaml
columns:
  - name: user_id
    type: INTEGER
  - name: email
    type: TEXT
  - name: signup_date
    type: DATE
```

You can then edit this file to add `description:` annotations, adjust types, or set `materialization: ephemeral`. smelt uses the pinned types instead of re-inferring from the CSV on every build.

The action is only offered when no sidecar exists. If a sidecar already exists and you need to update the column declarations, edit the `.yml` file directly.

### Hover on seed references

Hovering over a `smelt.<seed-address>` reference in a SQL model shows the seed's column names and inferred (or pinned) types. If a sidecar YAML is present, the pinned types are shown instead of the inferred ones.

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
