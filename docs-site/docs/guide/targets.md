# Targets and Backends

Targets are named execution environments defined in `smelt.yml`. Each target specifies a backend type (DuckDB, Spark) and connection details. You can define multiple targets and switch between them at runtime.

## Defining targets

Targets are listed under the `targets` key in `smelt.yml`:

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

  spark:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: main
```

The first target listed is not automatically the default -- smelt defaults to a target named `dev` unless you specify otherwise with `--target`.

## Backends

### DuckDB

DuckDB is an embedded analytical database. smelt bundles a DuckDB binary, so no separate installation is required.

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
```

| Field | Required | Description |
|---|---|---|
| `type` | Yes | Must be `duckdb`. |
| `database` | Yes | Path to the DuckDB database file. Created automatically if it does not exist. |
| `schema` | Yes | Default schema for created tables and views. |

DuckDB is the recommended backend for local development and testing. The database file is portable and can be inspected with the DuckDB CLI or any tool that supports DuckDB.

### Spark

Spark is supported via Spark Connect for distributed execution.

```yaml
targets:
  spark_prod:
    type: spark
    connect_url: sc://spark-cluster:15002
    catalog: spark_catalog
    schema: production
```

| Field | Required | Description |
|---|---|---|
| `type` | Yes | Must be `spark`. |
| `connect_url` | Yes | Spark Connect URL (e.g., `sc://host:15002`). |
| `catalog` | No | Spark catalog name. |
| `schema` | Yes | Default schema for created tables and views. |

## Switching targets

Use the `--target` flag on any command:

```bash
# Run against DuckDB (default)
smelt run

# Run against Spark
smelt run --target spark

# Build with a specific target
smelt build --target spark_prod

# Seed into a specific target
smelt seed --target dev
```

## Per-model target overrides

Individual models can be pinned to a specific target, regardless of the `--target` flag. This is useful in multi-engine setups where some models must run on a particular backend.

**In smelt.yml:**

```yaml
models:
  heavy_aggregation:
    target: spark_prod
  quick_lookup:
    target: dev
```

**In YAML frontmatter:**

```sql
---
target: spark_prod
---
SELECT ...
```

Target precedence (highest to lowest):

1. YAML frontmatter in the SQL file
2. `models:` section in `smelt.yml`
3. `--target` CLI flag (defaults to `dev`)

## Multi-target setup example

A typical project uses DuckDB for development and Spark for production:

```yaml
name: my_project
version: 1

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

  spark:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: main

models:
  # Most models use whatever target is passed via --target
  daily_revenue:
    materialization: table
  # This model always runs on Spark, even during dev
  large_aggregation:
    target: spark
    materialization: table
```

```bash
# Development: everything runs on DuckDB (except large_aggregation)
smelt build

# Production: everything runs on Spark
smelt build --target spark
```

## Cross-engine SQL compilation

smelt compiles SQL to the target's dialect automatically. You write standard SQL with `smelt.ref()` and `smelt.source()`, and smelt translates function calls, types, and syntax to match the target backend.

!!! note
    Not all SQL features are available on all backends. If you use a backend-specific function, smelt will report an error when targeting a backend that does not support it.

## Further reading

- [Materializations](materializations.md) for how tables and views are created in each target
- [Incremental Models](incremental-models.md) for time-partitioned processing across backends
