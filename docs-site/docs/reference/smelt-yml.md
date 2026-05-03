# Project Configuration (smelt.yml)

The `smelt.yml` file is the main configuration file for a smelt project. It must be located at the root of your project directory.

## Top-Level Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | yes | | Project name |
| `version` | integer | yes | | Configuration version (currently `1`) |
| `paths` | string[] | no | `["models"]` | Workspace-relative directories scanned for project files (`.sql`, `.py`, `.csv`, `.yml`). Kind is determined by file format/content, not by which directory the file lives in. |
| `targets` | map | yes | | Named execution environments (see [Targets](#targets)) |
| `default_materialization` | string | no | `"view"` | Default materialization for all models |
| `models` | map | no | `{}` | Per-model configuration overrides (see [Model Configuration](#model-configuration)) |
| `python` | string | no | | Path to Python interpreter. Can also be set via the `SMELT_PYTHON` environment variable, which takes precedence over this field. |

---

## Targets

Targets define execution environments. Each target has a name (the map key) and specifies a backend type with its connection details.

```yaml
targets:
  <target_name>:
    type: <backend_type>
    # backend-specific fields...
```

You can define multiple targets and select one at runtime with the `--target` CLI flag (default: `dev`).

### DuckDB Target

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Must be `duckdb` |
| `database` | string | yes | Path to the DuckDB database file (relative to project root) |
| `schema` | string | yes | Database schema to use |

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
```

### Spark Target

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Must be `spark` |
| `connect_url` | string | yes | Spark Connect URL (e.g., `sc://localhost:15002`) |
| `catalog` | string | no | Spark catalog name |
| `schema` | string | yes | Database schema to use |
| `format` | string | no | Table format: `delta` (default) or `parquet`. Affects schema evolution capabilities. See [Schema Evolution](../guide/schema-evolution.md#backend-capability-matrix). |

```yaml
targets:
  spark_prod:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: main
    format: delta  # default; can also be "parquet"
```

---

## Materialization Types

The `default_materialization` field and per-model `materialization` field accept these values:

| Value | Description |
|-------|-------------|
| `table` | Persisted as a physical table. Required for incremental models. |
| `view` | Created as a database view. Re-computed on each query. |
| `ephemeral` | Not materialized at all. Inlined as a CTE into downstream models. Cannot have incremental config or target overrides. |
| `materialized_view` | Backend-managed persistent view (e.g., PostgreSQL, Databricks). Refreshed atomically. |

**Precedence for materialization resolution:**

1. SQL file frontmatter (`materialization:` in the model file)
2. `smelt.yml` per-model config (`models.<name>.materialization`)
3. `smelt.yml` top-level `default_materialization`
4. Built-in default: `view`

When `materialization` is omitted at every level, a model is materialized as a `view`. See the [Materializations guide](../guide/materializations.md) for when to override this.

---

## Model Configuration

Per-model configuration is specified under the `models` key, using the model name (filename without extension) as the key.

```yaml
models:
  <model_name>:
    materialization: <type>
    tags: [<tag>, ...]
    target: <target_name>
    incremental:
      # incremental fields...
```

### Model Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `materialization` | string | no | _(project default)_ | Materialization type for this model |
| `tags` | string[] | no | `[]` | Tags for model selection (used with `--select tag:X`) |
| `target` | string | no | _(CLI default)_ | Override which target to execute this model on |
| `incremental` | object | no | | Incremental materialization configuration (see below) |
| `schema_evolution` | object | no | | Schema evolution configuration (see [Schema Evolution](#schema-evolution-configuration)) |
| `format` | string | no | _(from target)_ | Override the table format for this model: `delta` or `parquet`. Only relevant for Spark targets. |
| `columns` | map | no | `{}` | Per-column metadata: `default`, `backfill`, `description`, `tests` |

**Target precedence:** SQL file frontmatter > `smelt.yml` model config > CLI `--target` flag.

**Tags** from `smelt.yml` and SQL frontmatter are merged (union, deduplicated).

### Incremental Configuration

Incremental materialization processes only new or changed data instead of rebuilding the entire table. It is only valid for models with `materialization: table`.

```yaml
models:
  daily_revenue:
    materialization: table
    incremental:
      enabled: true
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
      unique_key:
        - transaction_id
      safety_overrides:
        allow_window_functions: false
```

#### Incremental Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `enabled` | bool | no | `true` | Whether incremental processing is active |
| `event_time_column` | string | yes | | Column in source data to filter on (used in the injected WHERE clause) |
| `partition_column` | string | yes | | Column in the output table to delete by (for DELETE+INSERT strategy) |
| `granularity` | string/object | yes | | Partition granularity (see [Granularity](#granularity)) |
| `unique_key` | string[] | no | `[]` | Columns that uniquely identify a row. When present, the backend may choose a MERGE strategy instead of DELETE+INSERT. |
| `safety_overrides` | object | no | _(all false)_ | Override safety checks for patterns that may produce different results on partial data (see [Safety Overrides](#safety-overrides)) |

#### Granularity

The `granularity` field controls the size of each partition window. It accepts the following values:

| Value | Description |
|-------|-------------|
| `hour` | Hourly partitions |
| `day` | Daily partitions |
| `week` | Weekly partitions (requires `week_start` subfield) |
| `month` | Monthly partitions |
| `quarter` | Quarterly partitions |
| `year` | Yearly partitions |

For weekly granularity, you must specify the start day:

```yaml
granularity:
  week:
    week_start: monday
```

Valid `week_start` values: `monday`, `tuesday`, `wednesday`, `thursday`, `friday`, `saturday`, `sunday`.

All other granularities are simple strings:

```yaml
granularity: day
```

#### Safety Overrides

Smelt validates incremental models to ensure they produce the same results whether run on the full dataset or on individual partitions. Certain SQL patterns can violate this guarantee. Safety overrides let you acknowledge and allow these patterns when you know they are safe for your use case.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `allow_window_functions` | bool | `false` | Allow window functions (e.g., `ROW_NUMBER()`, `LAG()`) which may produce different results on partial data |
| `allow_having` | bool | `false` | Allow HAVING clauses which filter on aggregates that may differ per-partition |
| `allow_limit` | bool | `false` | Allow LIMIT which produces non-deterministic results on partial data |
| `allow_subqueries` | bool | `false` | Allow subqueries which may reference data outside the current partition |
| `allow_nondeterministic` | bool | `false` | Allow nondeterministic functions (e.g., `RANDOM()`, `NOW()`) |
| `allow_distinct` | bool | `false` | Allow DISTINCT which may produce different results when data is split across partitions |

### Schema Evolution Configuration

Schema evolution controls how smelt handles changes to an incremental model's output schema. See the [Schema Evolution guide](../guide/schema-evolution.md) for detailed examples.

```yaml
models:
  my_model:
    materialization: table
    schema_evolution:
      strategy: alter_and_backfill
    columns:
      status:
        default: "'pending'"
        backfill: "CASE WHEN status IS NULL THEN 'pending' ELSE status END"
```

#### Schema Evolution Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `strategy` | string | `alter_and_backfill` | `alter_and_backfill`: use ALTER TABLE when possible. `full_refresh`: always drop and recreate on any schema change. |

#### Column Fields (for schema evolution)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default` | string | | SQL expression for the DEFAULT value when adding a column via ALTER TABLE. Must be a valid SQL expression (e.g., `"0"`, `"'unknown'"`, `"NULL"`, `"STRUCT_PACK(a := 0)"`). |
| `backfill` | string | | SQL expression for UPDATE backfill after a column is added. Used as: `UPDATE table SET column = <backfill_expr>`. |

---

## Validation Rules

Smelt validates model configurations and reports errors or warnings:

**Errors (block execution):**

- Ephemeral models cannot have incremental configuration
- Ephemeral models cannot have a target override

**Warnings (printed to stderr):**

- View models with incremental config (incremental only applies to tables)
- Materialized view models with incremental config (materialized views are refreshed atomically)

---

## Complete Example

The following is a fully annotated `smelt.yml` based on the timeseries example project:

```yaml
# Project identity
name: smelt_examples
version: 1

# Workspace-relative directories scanned for project files (.sql, .py, .csv, .yml).
# Default: ["models"]. Kind is determined by file format/content, not by directory.
paths:
  - models
  - seeds

# Execution environments
targets:
  # Local development with DuckDB (default target)
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

  # Remote Spark cluster
  spark:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: main

# Default materialization for models not explicitly configured
default_materialization: view

# Per-model configuration
models:
  # Simple table materialization
  users:
    materialization: table

  events:
    materialization: table

  user_activity:
    materialization: table

  transactions:
    materialization: table

  # Incremental model with full configuration
  daily_revenue:
    materialization: table
    incremental:
      enabled: true
      event_time_column: transaction_timestamp  # Column in source data (WHERE filter)
      partition_column: revenue_date             # Column in output (DELETE target)
      granularity: day

  cube_metrics:
    materialization: table
```
