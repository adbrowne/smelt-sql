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
| `maintenance` | object | no | | Project-level maintenance-plan baseline (today only `scan_bounds`); a per-model `maintenance:` block in SQL frontmatter refines it (see [Maintenance Configuration](#maintenance-configuration)) |

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

**State isolation per target.** Run state (interval coverage, reconciliation ledgers, deployed-schema snapshots, run history) is stored under `.smelt/targets/<target_name>/`, so each target has its own closed, disjoint state store — a `dev` run can never mask a coverage gap in `prod`, and vice versa. See `docs/reference/cli.md` §"State isolation per target" and `docs/specs/run_state.md` §"`.smelt/` directory layout" for the full on-disk shape.

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

### Environment interpolation

Any string value anywhere in `smelt.yml` may reference an environment variable with `${VAR_NAME}`. The reference is resolved once, at config load, before validation — this is how secrets (a Spark `connect_url`, a warehouse credential) stay out of the checked-in file. Write `$$` for a literal `$` that must not trigger a lookup.

```yaml
targets:
  spark_prod:
    type: spark
    connect_url: ${SPARK_CONNECT_URL}   # e.g. sc://spark.internal:15002 from CI secrets
    catalog: spark_catalog
    schema: main
```

If `SPARK_CONNECT_URL` is unset, loading the config fails immediately with an error naming both the variable and the key path (`targets.spark_prod.connect_url`) — it never silently resolves to an empty string. When a config references more than one missing variable, every one of them is reported together, not just the first.

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
    unique_key: [<column>, ...]
    batched:
      # batched fields...
```

`refresh: incremental` is admitted on the two **shape-defining facts** alone — a `timeseries:` block (the clock) and/or a top-level `unique_key:` (the identity). Declaring one or both is enough; declaring neither is a hard error naming what's missing. `grain:` itself is never required to admit a model — it is an optional, **check-only** `partition` / `key` / `key_per_partition` label the two facts derive; write it only when you want the friendly name in frontmatter, and it errors if it disagrees with what the facts derive.

### Model Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `materialization` | string | no | _(project default)_ | Materialization type for this model |
| `tags` | string[] | no | `[]` | Tags for model selection (used with `--select tag:X`) |
| `target` | string | no | _(CLI default)_ | Override which target to execute this model on |
| `timeseries` | object | no | | Time-dimension declaration — the **clock** shape-defining fact (see [Timeseries Configuration](#timeseries-configuration)) |
| `refresh` | string | no | `full` | Refresh axis: `full`, `incremental`, or `materialized_view` |
| `unique_key` | string \| string[] | no | | The **identity** shape-defining fact — the output's row identity. A single string is sugar for a one-element list. Together with `timeseries:`, this is what admits `refresh: incremental`; frontmatter wins over this `smelt.yml` override when both set it. Distinct from the `batched:` sub-block's `unique_key` (a partition-grain dedup aid, never identity). |
| `grain` | string | no | | Optional check-only assertion — `partition`, `key`, or `key_per_partition` — validated against the label `timeseries:`/`unique_key:` derive; never a driver |
| `batched` | object | no | | Preference/config block layered on top of `refresh: incremental` (see [Batched Configuration](#incremental-configuration)) |

**Target precedence:** SQL file frontmatter > `smelt.yml` model config > CLI `--target` flag.

**Tags** from `smelt.yml` and SQL frontmatter are merged (union, deduplicated).

!!! note "Layer split: smelt.yml vs SQL frontmatter"
    The fields listed above are the **complete set** of per-model configuration accepted in `smelt.yml`. Other per-model settings — `schema_evolution` (schema-change strategy), `columns` (per-column defaults and backfills), and per-model table `format` — are declared in the model's **SQL frontmatter**, not in `smelt.yml`. Placing these keys under `models.<name>:` in `smelt.yml` has no effect. See the [SQL Models guide](../guide/sql-models.md#supported-metadata-fields) and the [Schema Evolution guide](../guide/schema-evolution.md) for how to declare them.

### Timeseries Configuration

Models that process time-partitioned data must declare a `timeseries:` block. This is required for `refresh: incremental` + `grain: partition` models. A `grain: key` model *may also* declare `timeseries:` to time-partition its keyed output — the key and clock axes are independent, not alternatives — but only when key temporal locality is established (a proof, or a checked declaration, that every duplicate delivery of one key stays within a bounded window of itself on the event axis; see the [composed shape](../guide/incremental-models.md#the-composed-shape-key-time)). A `grain: key` model whose `timeseries:` block satisfies none of the three locality routes is refused (`KeyedForbidsTimeseries`, naming the missing route). The `timeseries:` and `batched:` keys are siblings, not nested.

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

### Maintenance Configuration

`maintenance:` is a sibling of `timeseries:`/`batched:` in SQL frontmatter (it is not a `smelt.yml` per-model field). It constrains the derived maintenance plan — the per-`(column-group × trigger)` cell matrix `smelt explain <model>` prints — without ever choosing a strategy the derivation didn't already admit.

```yaml
maintenance:
  scan_bounds:
    per_source:
      raw.users:
        allow_full_scan: true
```

The most common use is `scan_bounds.per_source.<source>.allow_full_scan: true`, which names your acceptance of a full read of an unclocked source (one with no `partition_column` to bound a scan by). Some maintenance cells — e.g. a column-scoped `MERGE` driven by a mutable dimension's `UpstreamMutation` trigger — can only be admitted by reading that dimension in full; without this acceptance, the plan refuses the cell and `smelt run` falls back to region-recompute (`DELETE`+`INSERT`) instead. See [Enrichment joins and dimension updates](../guide/incremental-models.md#enrichment-joins-and-dimension-updates) for the full example.

#### Maintenance Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `scan_bounds.require` | string | no | `partition_local` | The partition-locality guardrail: `partition_local` or `none`. |
| `scan_bounds.on_violation` | string | no | `error` | What to do when the derived plan exceeds the stated expectation: `error` or `warn`. |
| `scan_bounds.per_source.<address>.allow_full_scan` | bool | no | `false` | Named acceptance of a full (unbounded) read of the source at `<address>`. |

A project-level `maintenance.scan_bounds` block in `smelt.yml`'s top level sets the baseline; a per-model `maintenance:` block in the SQL frontmatter refines it (narrower wins).

`maintenance.defaults.prefer`, `maintenance.cells[].prefer`, and `maintenance.cells[].technique` primarily choose among the *techniques* a cell's derived plan admits (fold vs. region recompute vs. rederiving columns). This family choice is live: a `technique:` pin is a hard override that a run honours directly (bypassing the cost-model default, never bypassing admission — an inadmissible pin fails the run loudly, naming the cell), and `prefer:` nudges the same default without ever refusing. There is no separate config surface for this — the same keys drive both a live run's resolution and [`smelt bakeoff`](cli.md#smelt-bakeoff)'s offline measurement of what each admissible technique costs.

The same keys also carry a `suppress`/`unconditional` value that steers the orthogonal [conditional-write](../guide/incremental-models.md#conditional-writes) dimension: whether a `ColumnScopedMerge`/`KeyedFold` cell's matched arm is suppressed for unchanged rows. By default this follows a structural rule (a steady-state trigger prefers suppression; a first-build/backfill trigger prefers the plain matched arm), never bypassing the underlying row-identity/column-comparability proof — `prefer: suppress`/`prefer: unconditional` nudge the default without ever refusing, and `technique: suppress`/`technique: unconditional` force it, refusing loudly if the proof itself never admitted suppression. This suppression ladder only drives the live run path for `ColumnScopedMerge` cells today; a `KeyedFold` cell's `refresh: keyed` executor still always honours a proven `Suppressed` verdict unconditionally, regardless of trigger or override — `smelt explain` resolves and prints the ladder's answer for it, but that answer doesn't yet reach the live keyed-fold write.

#### Pinning a measured technique

[`smelt bakeoff <model> --pin`](cli.md#smelt-bakeoff) measures every admissible technique for a
cell against replayed windows of real data and prints the cheapest one as a ready-to-paste
`cells[]` entry — the same shape as the block above, with `technique:` set to the winner. The
command never edits the `.sql` file itself; paste the printed block into the model's
frontmatter yourself:

```yaml
maintenance:
  cells:
    - columns: [user_name]
      on: users
      technique: fold
```

Once pasted, the pin is an ordinary override, re-validated through admission on every compile —
if the plan later stops admitting that technique for the cell (e.g. a schema change removes the
proof it relied on), the next compile fails loud rather than silently reverting to the default.

#### `cells[].write` — the physical addressing pin

`maintenance.cells[].write` is a separate axis from `prefer`/`technique`: it pins *how* a cell physically locates the rows it writes (region `DELETE`+`INSERT`, keyed `MERGE`, column-scoped `MERGE`, in-place `UPDATE`, full rebuild, or a backend-contributed pattern), not which technique family runs.

```yaml
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      technique: recompute
      write: region
```

`write:` is an **open name**, not a sealed keyword set — it resolves against a registry of currently-known write patterns, and the set grows as backends contribute new ones. A pin is validated, never silently honoured or downgraded:

- An unrecognised pattern name, or one the model's target backend cannot execute (e.g. `write: column` on a backend with no column-scoped `MERGE`), fails the build with `MaintenanceWritePatternUnavailable`, naming the pattern and the backend.
- A recognised, backend-capable pattern that this cell's own declared facts cannot support (e.g. `write: keyed` on an output with no `unique_key`) fails with `MaintenanceWriteAddressingRefused`, naming the cell and the pattern.
- A refused pin never falls back to a different addressing — fix the pin or the model's declared facts.

`smelt explain <model>` prints each cell's admissible pattern set and its active pin (if any), so you can see what a pin would resolve against before setting one.

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
