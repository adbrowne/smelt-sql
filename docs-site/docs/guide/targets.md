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
| `settings` | No | Map of DuckDB connection settings applied on open. See below. |

DuckDB is the recommended backend for local development and testing. The database file is portable and can be inspected with the DuckDB CLI or any tool that supports DuckDB.

#### DuckDB `settings`

The optional `settings:` map applies DuckDB configuration keys immediately after the connection opens, before any model executes. Each entry becomes a `SET key = value` statement. Unknown keys are rejected with an error at startup.

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
    settings:
      memory_limit: "4GB"
      threads: "4"
      temp_directory: /tmp/duckdb_scratch
```

Common settings:

| Key | Description |
|---|---|
| `memory_limit` | Maximum memory DuckDB may use (e.g. `"1GB"`, `"512MB"`). |
| `threads` | Number of worker threads for parallel query execution. |
| `temp_directory` | Directory for temporary spill files when memory is exceeded. |

For the full list of DuckDB settings, see the [DuckDB configuration reference](https://duckdb.org/docs/configuration/overview).

### Spark

Spark is supported via Spark Connect for distributed execution. smelt compiles the same logical
models to Spark SQL, handling dialect differences (QUALIFY rewrites, date literal forms, `::` cast
lowerings) automatically. The smelt web UI also supports Spark targets — select a Spark target in
the UI and models run on the connected server.

```yaml
targets:
  spark_prod:
    type: spark
    connect_url: sc://spark-cluster:15002
    catalog: spark_catalog
    schema: production
    format: delta  # default; use "parquet" for reduced-capability clusters
```

| Field | Required | Description |
|---|---|---|
| `type` | Yes | Must be `spark`. |
| `connect_url` | Yes | Spark Connect URL (e.g., `sc://host:15002`). |
| `catalog` | No | Spark catalog name. |
| `schema` | Yes | Default schema for created tables and views. |
| `format` | No | Table format: `delta` (default) or `parquet`. See [Delta vs Parquet](#delta-vs-parquet) below. |

#### Secrets and TLS

`connect_url` accepts environment-variable interpolation, so an auth token never has to sit in
the checked-in `smelt.yml`. Any `${VAR_NAME}` reference inside the string is resolved once at
config load, from the process environment; a literal `$` that must not trigger a lookup is
written `$$`. If the referenced variable is unset, config loading fails with a hard error naming
the variable and the YAML key path (e.g. `targets.prod.connect_url`) — it never silently
resolves to an empty string.

```yaml
targets:
  databricks_prod:
    type: spark
    connect_url: "sc://adb-123.4.azuredatabricks.net:443/;token=${DATABRICKS_TOKEN};use_ssl=true"
    catalog: main
    schema: analytics
```

TLS and other connection parameters (`use_ssl`, `token`, etc.) are passed the same way, as part
of the Spark Connect URL string — smelt does not parse them out or introduce separate YAML keys
for them. The resolved URL, token included, is handed to the Spark Connect Python client
unmodified and is never logged.

A `connect_url` holding a literal (non-`${VAR}`) token is a lint-worthy smell: the secret sits in
the committed YAML in plaintext, which is exactly what interpolation exists to avoid.

#### Delta vs Parquet

The `format:` field selects the Spark table format, which determines which capabilities are available:

| Capability | Delta | Parquet |
|---|:---:|:---:|
| MERGE (incremental) | ✓ | ✗ |
| Column mapping / schema evolution | ✓ | ✗ |
| `supports_nested_array_ddl` | ✓ | ✗ |
| `supports_struct_field_ddl` | ✓ | ✗ |
| `supports_merge_schema_write` | ✓ | ✓ |

**Delta is the default and the parity baseline.** MERGE-based incremental models and rich schema
evolution both require Delta. Use `format: parquet` only on clusters where Delta Lake is not
available; doing so restricts the available incremental strategies and disables column mapping.

To use Delta, ensure your Spark cluster has Delta Lake installed (e.g. the
`io.delta:delta-spark_2.13:4.0.0` package). See `scripts/spark-up.sh` for the reference setup
used in CI.

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

## Spark requirements

The Spark backend communicates via PySpark over Spark Connect. You need:

- **Python** with PySpark installed (`pip install pyspark`)
- **Spark Connect server** running on the configured URL
- For **Databricks**: use `pip install databricks-connect` instead of `pyspark`
- For **EMR/Dataproc**: ensure Spark Connect is enabled on the cluster

smelt uses PyO3 to call PySpark from Rust. Data is exchanged via Arrow (zero-copy), so there is no serialization overhead for query results.

### Spark CI coverage

A pull request touching Spark-relevant code (the Spark backend crate, Spark/parity integration
tests, the function-signature registry, type inference, the parser's dialect surface, or the
Python adapter) automatically runs the Spark parity suite and the Spark type-property suite
against a live Delta-enabled Spark Connect server before merge. Every other PR gets the full
Spark job set on the next nightly run, so a regression outside that path filter still surfaces
within one cycle rather than sitting unnoticed on `main`.

### Known limitations

Full-refresh and view materializations, ephemeral models, and the `batched`/`keyed`/`versioned`
incremental maintenance techniques are verified on Spark by the same parametrized tests that run
against DuckDB, plus hand-authored fixed-recipe dual-target parity tests per technique. The
generative incremental-maintenance sweep (randomized recipe pool, admission-rate statistics,
DAG-propagation, composed-pool, pinned-hazard, and change-feed-admission legs) also runs against
a live Spark Connect server in the gated CI tier, driven by the same recipe pool and
multiset-equivalence oracle as the DuckDB leg. What is **not** covered by that sweep on Spark:

| Area | Status |
|---|---|
| `Additive`-combiner keyed/composed folds (e.g. `SUM` across a keyed or composed cumulative fold) | No Spark ledger dialect yet for the never-fold-twice reconciliation ledger; the runtime fails loud rather than silently mishandling it |
| Feed-declared source recompute, replayed against a change-log oracle (admission is covered) | Oracle-replay machinery is DuckDB-connection-specific; execution-driven leg not yet ported |
| Probe harness (`window_order_permutations_converge`, write-window byte-equality, technique-pin agreement) | Staging/read-back is DuckDB-connection-specific; not yet generalized to the backend trait |
| Skeleton-position-add refusal path | No Spark fixture yet |
| Partition pruning on cross-engine `read_parquet()` reads | Not implemented — every downstream run reads the full Parquet glob (performance gap, not correctness) |
| Databricks-specific capabilities | Not modeled as a distinct backend; Databricks Connect works via the generic Spark Connect adapter, but Databricks-only behavior isn't verified |

## Cross-engine data exchange

When models on different backends reference each other, smelt automatically handles data transfer via Parquet files.

**How it works:**

1. A Spark model writes its output as Parquet files in the warehouse directory
2. A DuckDB model references the Spark model with `smelt.spark_model`
3. smelt resolves the cross-engine reference and emits a `read_parquet()` call pointing to the Spark model's output files
4. DuckDB natively reads the Parquet files -- no explicit copy step

**Example:**

```yaml
# smelt.yml
targets:
  local:
    type: duckdb
    database: target/dev.duckdb
    schema: main
  spark:
    type: spark
    connect_url: sc://localhost:15002
    schema: analytics

models:
  # Runs on Spark
  heavy_transform:
    target: spark
    materialization: table

  # Runs on DuckDB, reads from Spark output
  reporting_summary:
    materialization: table
```

```sql
-- models/reporting_summary.sql
-- This ref resolves to read_parquet('warehouse/analytics/heavy_transform/**/*.parquet')
SELECT category, SUM(amount) as total
FROM smelt.heavy_transform
GROUP BY 1
```

!!! note
    Cross-engine exchange currently uses the local filesystem. Cloud storage (S3, GCS, ADLS) is not yet supported.

## Cross-engine SQL compilation

smelt compiles SQL to the target's dialect automatically. You write standard SQL with `smelt.<name>` and `smelt.sources.<name>`, and smelt translates function calls, types, and syntax to match the target backend.

!!! note
    Not all SQL features are available on all backends. If you use a backend-specific function, smelt will report an error when targeting a backend that does not support it.

## Further reading

- [Materializations](materializations.md) for how tables and views are created in each target
- [Incremental Models](incremental-models.md) for time-partitioned processing across backends
