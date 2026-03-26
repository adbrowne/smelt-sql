# Project Structure

## Directory Layout

A typical smelt project looks like this:

```
my-project/
├── smelt.yml            # Project configuration
├── sources.yml          # External source definitions (optional)
├── models/              # SQL and Python model files
│   ├── staging/         # Raw data cleanup
│   ├── intermediate/    # Business logic
│   └── marts/           # Final analytical tables
├── seeds/               # CSV files loaded as tables
│   └── raw/             # Subdirectories map to schemas
└── target/              # Generated artifacts (gitignored)
    └── dev.duckdb       # DuckDB database file
```

## smelt.yml

The project configuration file defines your project name, model paths, seed paths, target environments, and default materialization strategy.

```yaml
name: my-project
version: 1

model_paths:
  - models

seed_paths:
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

Models reference sources with `smelt.source('raw.page_views')`.

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
FROM smelt.ref('staging.orders')
WHERE status = 'completed'
```

Subdirectories are purely organizational -- they have no semantic meaning. Use whatever structure makes sense for your team. A common convention is:

- **staging/** -- Light transformations that clean raw source data (renaming columns, casting types, filtering invalid rows).
- **intermediate/** -- Business logic that combines staging models. Not typically exposed to end users.
- **marts/** -- Final analytical tables consumed by dashboards and reports.

Python models (`.py` files with a `@model` decorator) are also supported in the models directory. See the [Python models guide](../guide/python-models.md) for details.

## seeds/ Directory

CSV files in `seeds/` are loaded into the database as tables. This is useful for small reference data like country codes, category mappings, or test fixtures.

```
seeds/
├── raw/
│   ├── country_codes.csv
│   └── currency_rates.csv
└── lookups/
    └── status_mapping.csv
```

Subdirectory names become schema names in the database:

| File path | Database table |
|---|---|
| `seeds/raw/country_codes.csv` | `raw.country_codes` |
| `seeds/lookups/status_mapping.csv` | `lookups.status_mapping` |

Models reference seeds the same way they reference other models: `smelt.ref('raw.country_codes')`.

See the [seeds guide](../guide/seeds.md) for more details.

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
    The `target/` directory is fully managed by smelt. You can safely delete it to start fresh -- smelt will recreate everything on the next run (incremental models will do a full refresh).

## Example Projects

The smelt repository includes complete example projects you can use as references:

[**examples/timeseries/**](https://github.com/adbrowne/smelt-sql/tree/main/examples/timeseries)
:   A user and event analytics pipeline with 12 SQL models. Demonstrates incremental materialization, seed data, and interval-based processing.

[**examples/retail_analytics/**](https://github.com/adbrowne/smelt-sql/tree/main/examples/retail_analytics)
:   A TPC-DS-based retail analytics pipeline with 25 models organized in staging, intermediate, and marts layers. A good reference for larger project structure.

To try an example locally:

```bash
cd examples/timeseries
smelt run
```
