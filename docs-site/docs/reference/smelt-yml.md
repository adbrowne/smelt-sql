# Project Configuration (smelt.yml)

The `smelt.yml` file is the main configuration file for a smelt project. It must be located at the root of your project directory.

## Top-Level Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | yes | | Project name |
| `version` | integer | no | `1` | Configuration version (currently `1`). Defaults to `1` when omitted. |
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
| `settings` | map | no | Connection-time DuckDB settings applied as `SET key = value` on open (e.g. `memory_limit`, `threads`, `temp_directory`). Unknown keys are rejected with an error. |

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
    settings:
      memory_limit: "16GB"          # cap DuckDB's buffer pool
      temp_directory: target/spill  # where to spill when over the limit
```

#### Memory limits and spilling

Left to itself, DuckDB sizes its buffer pool at **~80% of total host RAM** and
only spills to disk once it reaches that ceiling. On a machine shared with other
work (your editor, a language server, a second build, an agent), one heavy model
scanning a large source can grow toward that ceiling and push the whole host into
memory pressure.

To keep smelt a good citizen by default, when a DuckDB target's `settings:` omits
these keys smelt fills them in:

- **`memory_limit`** — defaulted to roughly `min(50% of RAM, RAM − 20 GB)` (floored
  at 40% of RAM on small hosts), leaving generous headroom for the OS and other
  processes. It is set conservatively because DuckDB's limit caps its buffer pool,
  not total process memory — actual RSS runs a few GB higher.
- **`temp_directory`** — defaulted to `.smelt-duckdb-tmp` next to your `.duckdb`
  file, so queries that exceed the limit **spill to disk** instead of failing or
  growing unbounded.

Any value you set yourself is used **exactly as written and never overridden** —
set `memory_limit: "48GB"` on a dedicated box to opt back into a larger budget, or
point `temp_directory` at a faster disk. `threads` is left at DuckDB's default.

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

Test files are identified by a `smelt.test` declaration in the SQL file, not by a materialization value. See [Testing](../guide/testing.md) for details.

A key-grain running-state table is opted in with `materialization: table` + `refresh: incremental` + `grain: key` — see [Materializations](../guide/materializations.md#refresh-axis) for details.

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
    refresh: incremental
    grain: partition
    batched:
      # batched fields...
```

### Model Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `materialization` | string | no | _(project default)_ | Materialization type for this model |
| `tags` | string[] | no | `[]` | Tags for model selection (used with `--select tag:X`) |
| `target` | string | no | _(CLI default)_ | Override which target to execute this model on |
| `timeseries` | object | no | | Time-dimension declaration for `grain: partition` / `grain: key_per_partition` models, or key-temporal-locality-admitted `grain: key` models (see [Timeseries Configuration](#timeseries-configuration)) |
| `refresh` | string | no | `full` | Refresh axis: `full`, `incremental`, or `materialized_view` |
| `grain` | string | no | | Required with `refresh: incremental`: `partition`, `key`, or `key_per_partition` |
| `batched` | object | no | | Preference/config block layered on top of `refresh: incremental` (see [Batched Configuration](#incremental-configuration)) |

**Target precedence:** SQL file frontmatter > `smelt.yml` model config > CLI `--target` flag.

**Tags** from `smelt.yml` and SQL frontmatter are merged (union, deduplicated).

!!! note "Layer split: smelt.yml vs SQL frontmatter"
    The fields listed above are the **complete set** of per-model configuration accepted in `smelt.yml`. Other per-model settings — `schema_evolution` (schema-change strategy), `columns` (per-column defaults and backfills), and per-model table `format` — are declared in the model's **SQL frontmatter**, not in `smelt.yml`. Placing these keys under `models.<name>:` in `smelt.yml` has no effect. See the [SQL Models guide](../guide/sql-models.md#supported-metadata-fields) and the [Schema Evolution guide](../guide/schema-evolution.md) for how to declare them.

### Timeseries Configuration

Models that process time-partitioned data must declare a `timeseries:` block. This is required for `refresh: incremental` + `grain: partition` models (bare `grain: key` models do not declare `timeseries:` unless key temporal locality is established). The `timeseries:` and `batched:` keys are siblings, not nested.

```yaml
models:
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: transaction_timestamp  # column in SOURCE data (WHERE filter)
      partition_column: revenue_date             # column in OUTPUT (DELETE target)
      granularity: day
    batched:
      unique_key:
        - transaction_id
```

#### Timeseries Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `event_time_column` | string | yes | | Column in source data to filter on (used in the injected WHERE clause). Must be a timestamp or date. |
| `partition_column` | string | yes | | Column in the output table to delete by (for DELETE+INSERT strategy). |
| `granularity` | string | yes | | Partition granularity: `hour`, `day`, `week`, `month`, `quarter`, or `year`. |
| `week_start` | string | no | | Start day for weekly partitions. Required when `granularity: week`. One of: `monday`, `tuesday`, `wednesday`, `thursday`, `friday`, `saturday`, `sunday`. |

Example with weekly granularity:

```yaml
models:
  weekly_rollup:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: event_ts
      partition_column: week_start_date
      granularity: week
      week_start: monday
```

### Incremental Configuration

`refresh: incremental` + `grain: partition` processes only new or changed data instead of rebuilding the entire table. This implies a stored `table`. A `timeseries:` block must also be present (see above); the `batched:` block itself is optional.

```yaml
models:
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: transaction_timestamp
      partition_column: revenue_date
      granularity: day
    batched:
      unique_key:
        - transaction_id
      safety_overrides:
        allow_window_functions: false
```

#### Incremental Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `unique_key` | string[] | no | `[]` | Columns that uniquely identify a row. When present, the backend may choose a MERGE strategy instead of DELETE+INSERT. |
| `nondeterministic_columns` | string[] | no | `[]` | Output columns exempt from the determinism requirement (e.g. `inserted_at = NOW()`). See [Non-deterministic columns](../guide/incremental-models.md#non-deterministic-columns). |
| `safety_overrides` | object | no | _(all false)_ | Override safety checks for patterns that may produce different results on partial data (see [Safety Overrides](#safety-overrides)) |

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

---

## Validation Rules

Smelt validates model configurations and reports errors or warnings:

**Errors (block execution):**

- Ephemeral models cannot declare `refresh: incremental` / `grain:` / a `batched:` block
- Ephemeral models cannot have a target override

**Warnings (printed to stderr):**

- View models with `refresh:` set (the refresh axis only applies to stored tables)
- `refresh: materialized_view` models with a `batched:` block (materialized views are refreshed atomically by the engine, not smelt's incremental loop)

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

  # Incremental model (grain: partition) — timeseries: and batched: are sibling keys
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: transaction_timestamp  # column in SOURCE data (WHERE filter)
      partition_column: revenue_date             # column in OUTPUT (DELETE target)
      granularity: day

  cube_metrics:
    materialization: table
```
