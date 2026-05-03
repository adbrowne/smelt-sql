# Project Structure

## Directory Layout

A typical smelt project looks like this:

```
my-project/
├── smelt.yml            # Project configuration
├── models/              # SQL model files (listed in paths:)
│   ├── staging/         # Raw data cleanup
│   ├── intermediate/    # Business logic
│   └── marts/           # Final analytical tables
├── sources.yml          # Aggregate source definitions (legacy; per-entity .yml preferred)
├── tests/               # Model test files
│   ├── test_user_activity.sql
│   └── test_cohort_sizes.sql
├── seeds/               # CSV files loaded as tables (may also be listed in paths:)
│   └── raw/             # Subdirectories are part of the address, not the schema
├── .smelt/              # Deployed schema state (gitignored)
│   └── schemas/         # Last-deployed column schemas per model
└── target/              # Generated artifacts (gitignored)
    └── dev.duckdb       # DuckDB database file
```

## smelt.yml

The project configuration file defines your project name, scan paths, target environments, and default materialization strategy.

```yaml
name: my-project
version: 1

paths:
  - models
  - seeds

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
  prod:
    type: spark
    connect_url: sc://spark-cluster.internal:15002
    catalog: spark_catalog
    schema: analytics

default_materialization: view
```

For full configuration options, see the [project configuration reference](../reference/smelt-yml.md).

## Kind-by-content classification

smelt determines the **kind** of every file by its format and content, not by its directory name. Any directory listed in `paths:` is scanned and each file is classified:

| File format | Content / sibling presence | Kind |
|---|---|---|
| `.sql` | bare SELECT | model |
| `.sql` | `smelt.define` declaration | function |
| `.sql` | `smelt.test` declaration | test |
| `.csv` | (any) | seed |
| `.yml` | no sibling `.csv` with the same stem | source |
| `.yml` | sibling `.csv` with the same stem exists | sidecar (not addressable; attaches schema to the seed) |

This means you can organise models, seeds, and sources in any directory structure you like — smelt does not use the directory name to decide what a file is.

## Addressing: scan-root prefix is stripped

Every entity is addressed as `smelt.<path>`. The path is the workspace-relative location of the file **with the matching `paths:` scan-root prefix stripped**.

Under `paths: ["models"]`:

| Filesystem location | Address |
|---|---|
| `models/staging/orders.sql` | `smelt.staging.orders` |
| `models/marts/customers.sql` | `smelt.marts.customers` |
| `models/data/users.csv` | `smelt.data.users` |
| `models/external/api/orders.yml` (no sibling `.csv`) | `smelt.external.api.orders` |

**Address uniqueness is global across all `paths:` directories.** If `paths: ["models", "fixtures"]` and both `models/users.csv` and `fixtures/users.csv` exist, smelt reports a workspace-load error — each address must map to exactly one file.

## Default DB-name mapping

A persisted entity addressed as `smelt.<path>` materialises in the database at:

- **Schema**: the active target's `schema:` (default `main`)
- **Table**: address path segments joined with `_`

| Address | Default DB location (schema = `main`) |
|---|---|
| `smelt.users` | `main.users` |
| `smelt.staging.orders` | `main.staging_orders` |
| `smelt.data.lookup.regions` | `main.data_lookup_regions` |

This does **not** apply to functions (inlined), ephemeral models/seeds (CTE), or externs (no path).

## sources.yml

Source definitions declare external tables that your models read from. Defining sources enables schema validation and LSP support (type checking, autocompletion).

```yaml
version: 1

sources:
  raw:
    tables:
      page_views:
        description: Raw page view events
        columns:
          - name: user_id
            type: INTEGER
          - name: url
            type: VARCHAR
          - name: viewed_at
            type: TIMESTAMP
```

Models reference sources with `smelt.sources.raw.page_views`.

For full configuration options, see the [sources configuration reference](../reference/sources-yml.md).

## models/ Directory

Each `.sql` file in `models/` defines one model. Models can include optional YAML frontmatter for configuration:

```sql
---
materialization: table
tags:
  - finance
  - daily
---
SELECT
    order_id,
    customer_id,
    amount
FROM smelt.staging.orders
WHERE status = 'completed'
```

Subdirectories are purely organizational — they have no semantic meaning beyond becoming part of the address and default DB name. Use whatever structure makes sense for your team. A common convention is:

- **staging/** — Light transformations that clean raw source data (renaming columns, casting types, filtering invalid rows).
- **intermediate/** — Business logic that combines staging models. Not typically exposed to end users.
- **marts/** — Final analytical tables consumed by dashboards and reports.

Python models (`.py` files with a `@model` decorator) are also supported in the models directory. See the [Python models guide](../guide/python-models.md) for details.

## seeds/ Directory

CSV files discoverable under `paths:` are loaded into the database as tables. This is useful for small reference data like country codes, category mappings, or test fixtures.

```
seeds/
├── raw/
│   ├── country_codes.csv
│   └── currency_rates.csv
└── lookups/
    └── status_mapping.csv
```

The address and default DB name use the path-join rule:

| File path | Address | Default DB table |
|---|---|---|
| `seeds/raw/country_codes.csv` | `smelt.raw.country_codes` | `main.raw_country_codes` |
| `seeds/lookups/status_mapping.csv` | `smelt.lookups.status_mapping` | `main.lookups_status_mapping` |

Models reference seeds using the address: `FROM smelt.raw.country_codes`.

See the [seeds guide](../guide/seeds.md) for more details.

## tests/ Directory

Test files are `.sql` files with `materialization: test` in YAML frontmatter. By convention, they live in a `tests/` directory, which must be listed in `paths` in your `smelt.yml`:

```yaml
paths:
  - models
  - tests
```

```
tests/
├── test_user_activity.sql
├── test_cohort_sizes.sql
└── test_customer_quantiles.sql
```

Each test defines mock inputs and expected outputs for a model (or a specific CTE within a model). Run tests with `smelt test`.

Tests can also be co-located as additional sections in model files — see the [Testing guide](../guide/testing.md) for details.

## target/ Directory

The `target/` directory contains generated artifacts and should be added to `.gitignore`.

For DuckDB targets, this is where the database file lives. smelt also stores run history and interval tracking state here for incremental models.

```
target/
├── dev.duckdb           # DuckDB database
├── run_history.json     # Execution log
└── intervals/           # Incremental model state
    └── staging.events.json
```

!!! note
    The `target/` directory is fully managed by smelt. You can safely delete it to start fresh — smelt will recreate everything on the next run (incremental models will do a full refresh).

## Example Projects

The smelt repository includes complete example projects you can use as references:

[**examples/timeseries/**](https://github.com/adbrowne/smelt-sql/tree/main/examples/timeseries)
:   A user and event analytics pipeline with 12 SQL models. Demonstrates incremental materialization, seed data, and interval-based processing.

[**examples/retail_analytics/**](https://github.com/adbrowne/smelt-sql/tree/main/examples/retail_analytics)
:   A TPC-DS-based retail analytics pipeline with 25 models organized in staging, intermediate, and marts layers. A good reference for larger project structure.

[**examples/multi_engine/**](https://github.com/adbrowne/smelt-sql/tree/main/examples/multi_engine)
:   A multi-backend project demonstrating Spark and DuckDB models in the same pipeline with cross-engine Parquet data exchange.

To try an example locally:

```bash
cd examples/timeseries
smelt run
```
