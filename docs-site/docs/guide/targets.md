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

Schema changes migrate in place for the additive cases — adding a nullable column, adding a
struct field, relaxing `NOT NULL` — in Spark's own spelling (`ADD COLUMNS (c STRING)`, a
three-part table name). Changes the deployed table cannot express — adding a `NOT NULL` column,
tightening to `NOT NULL`, dropping a column, or widening one — rewrite the table (Delta) or are
refused with a message naming the column and the limitation (Parquet), and need
`--allow-full-refresh`. Dropping and widening are refused because they require a Delta table
feature (`delta.columnMapping.mode`, `delta.enableTypeWidening`) that smelt does not enable, since
turning one on irreversibly raises the table's protocol version. See
[Schema evolution](schema-evolution.md#backend-capability-matrix) for the full per-operation
matrix.

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

### BigQuery

BigQuery is supported through Google's BigQuery client. A BigQuery dataset is the analogue of a
schema, so a target names a `project`, a `dataset`, and the dataset's `location` in place of
DuckDB's `database` or Spark's `connect_url`.

```yaml
targets:
  bigquery_prod:
    type: bigquery
    project: my-gcp-project
    dataset: analytics
    location: US
    schema: analytics
```

| Field | Required | Description |
|---|---|---|
| `type` | Yes | Must be `bigquery`. |
| `project` | Yes | GCP project the jobs are billed to. |
| `dataset` | No | Dataset holding the created tables and views. Defaults to `schema`. |
| `location` | No | Dataset location (e.g. `US`, `europe-west2`). Must match at query time. |
| `schema` | Yes | Default schema for created tables and views. |

smelt compiles the same logical models to GoogleSQL, handling dialect differences automatically —
`x::T` casts become `CAST(x AS T)`, partition replacement becomes a scoped `DELETE` + `INSERT`
(BigQuery has no `INSERT OVERWRITE`), and type names GoogleSQL does not recognise (`VARCHAR`,
`TEXT`, `DOUBLE`) are emitted as `STRING` and `FLOAT64`.

Schema changes migrate in place for the flat cases — adding a column, dropping one, widening a
scalar type, relaxing `NOT NULL` — in GoogleSQL's own spelling (`ALTER COLUMN … SET DATA TYPE`,
`NUMERIC(p,s)`, `INT64`). Changes GoogleSQL cannot express — anything inside a struct or array,
adding a `NOT NULL` column, or widening a column that is already `NOT NULL` — are refused with a
message naming the column and the limitation, and need `--allow-full-refresh` to rebuild the
model instead. See [Schema evolution](schema-evolution.md#backend-capability-matrix) for the full
per-operation matrix.

BigQuery is verified against a live warehouse: the fixed-recipe parity suites (materialization,
seeding, dialect lowering, pipe syntax, `MERGE`, incremental refresh, schema evolution) run against a real
BigQuery project, and incremental-model correctness is additionally checked generatively — the
same recipe pool, run schedules, and equivalence oracle the other backends use, parametrized to
target BigQuery. Every case in that generative suite passes against a live warehouse. Both suites
run locally against your own GCP project rather than in CI, since that keeps cloud credentials out
of the build pipeline — which also means a BigQuery regression surfaces when someone runs the
suites by hand, not on a schedule.

#### Credentials

The BigQuery backend authenticates from a short-lived OAuth access token read from
`SMELT_BQ_ACCESS_TOKEN`, and **never** falls back to Google application-default credentials. This
is deliberate: ambient credentials on a developer machine carry that developer's entire cloud
identity, so refusing the fallback keeps the explicitly-supplied token the only route to the
warehouse. A run with no token set fails with a message naming the token, rather than silently
picking up whichever identity happens to be logged in.

```bash
export SMELT_BQ_ACCESS_TOKEN="$(gcloud auth print-access-token)"
smelt run --target bigquery_prod
```

Prefer a service account scoped to the datasets it needs over a user credential.

### Databricks

A `databricks` target reaches a Databricks workspace through Databricks Connect — a serverless
compute session, not a SQL warehouse or a plain Spark Connect URL. It is specified against
Databricks **Free Edition**'s constraints: serverless-only compute and mandatory Unity Catalog,
so a target names a `catalog` and `schema` (Unity Catalog's addressing) rather than Spark's
`connect_url`/`warehouse`/`format`.

```yaml
targets:
  databricks_prod:
    type: databricks
    host: my-workspace.cloud.databricks.com
    token: ${SMELT_DBX_TOKEN}
    catalog: workspace
    schema: analytics
```

| Field | Required | Description |
|---|---|---|
| `type` | Yes | Must be `databricks`. |
| `host` | Required unless `token` is also absent | Workspace hostname. Must be a **bare hostname** — no scheme, no trailing slash (e.g. `my-workspace.cloud.databricks.com`, not `https://my-workspace.cloud.databricks.com/`). Absent together with `token` is the ambient form (see "Deployment" below); `host` absent with `token` present is refused, naming both keys. |
| `token` | No | A `${ENV}` reference to a service-principal secret or personal access token. **Must** be a `${VAR}` reference — a literal token value is a hard configuration error, never a warning, because the value is a whole-workspace credential that would otherwise sit in a checked-in file. When absent, the session authenticates with the client's ambient Databricks credentials instead. |
| `catalog` | No | Unity Catalog catalog name. Defaults to `workspace`. |
| `schema` | Yes | Unity Catalog schema holding created tables and views. |

A `databricks` target hard-errors, naming both the offending key and the backend, on any key
belonging to another backend's shape: `connect_url`, `warehouse`, `format`, `database`,
`settings`, `project`, `dataset`, and `location`. These are never silently ignored — Free
Edition's serverless compute has no host-visible warehouse directory and no format choice, so a
silently-dropped `warehouse:` would lose a user's intent rather than reject it.

smelt compiles the same logical models against Databricks using the SparkSQL dialect, with
Databricks-specific spellings layered on top where Unity Catalog diverges from vanilla Spark
(e.g. its own `DROP_COMMAND_TYPE_MISMATCH` error text on a self-referential bootstrap model).
Loading data into a `databricks` target goes through the backend's own Arrow load path — never a
host-path file the serverless session cannot see, since there is no persistent local filesystem
to hand it one.

There is no cross-engine data exchange into or out of a `databricks` target today — the
Parquet-glob substitution other backend pairs use has no Volumes-based equivalent yet.

Databricks is verified against a live Free Edition workspace: a full refresh, eleven consecutive
incremental windows, a dual-target parity sweep against DuckDB, and a full-refresh-oracle
equivalence check all ran against `examples/github_activity`'s 16-model pipeline. See
[`docs/handoffs/2026-09-13-databricks-findings.md`](https://github.com/adbrowne/smelt-sql/blob/main/docs/handoffs/2026-09-13-databricks-findings.md)
for the full findings, including the registered divergences and what remains open.

#### Credentials

The Databricks backend authenticates with a `${ENV}`-supplied token (service-principal OAuth
machine-to-machine or a personal access token) or, when `token` is omitted, the client's ambient
Databricks credentials — never a literal value in `smelt.yml`.

```bash
export SMELT_DBX_TOKEN="$(cat /path/to/minted/token)"
smelt run --target databricks_prod
```

#### Free Edition constraints

Databricks Free Edition carries no bill, so in place of a budget cap these are the measured
quotas that bound a project targeting it:

- **Serverless-only, Unity-Catalog-mandatory.** No SQL warehouse path, no format choice — this
  is why `warehouse` and `format` are refused keys rather than tolerated-but-ignored ones.
- **Max 5 concurrent job tasks per account**, one SQL warehouse capped at `2X-Small`.
- **No fixed storage GB cap** — governed by an account-wide fair-usage policy instead; exceeding
  it suspends compute rather than deleting data.
- **No published session idle timeout.** A serverless session can be torn down server-side on
  the order of single-digit seconds after the last statement — harmless (a `UserWarning`, not a
  failure) but worth expecting if you see `INVALID_HANDLE.SESSION_CLOSED` in logs.
- **A succession-grain incremental model rebuilds from the whole source on every window**, not
  just the window, because Databricks/Delta has no realisable tombstone ledger for the
  window-forward patch route — correct, but O(source) per window rather than O(window). See
  `docs/specs/state.md` §"The degradation contract".

#### Deployment: Databricks Asset Bundle

A `databricks` target can also run unattended, entirely on the platform, deployed as a
[Databricks Asset Bundle](https://docs.databricks.com/en/dev-tools/bundles/index.html). The
bundle declares one job with a daily schedule, a serverless environment for every task, and two
tasks in order: a loader task that lands the next day's data, then a `smelt run` task that
processes it as a genuine incremental window. `scripts/dbx-bundle.sh` is the only caller of
`databricks bundle validate`, `databricks bundle deploy` and `databricks bundle run`:

```bash
mise run setup-databricks               # pins and installs the Databricks CLI
bash scripts/dbx-bundle.sh validate     # databricks bundle validate — schema-checks the
                                         # bundle against a local stub; needs no workspace
bash scripts/dbx-bundle.sh deploy       # databricks bundle deploy — uploads the bundle and
                                         # the locally-built wheel
bash scripts/dbx-bundle.sh seed         # copies smelt.yml and models/ onto the Volume —
                                         # never .smelt/, so it cannot reset run state
bash scripts/dbx-bundle.sh run github_activity_daily   # databricks bundle run
```

smelt reaches the job as a wheel declared in the bundle's `artifacts:` block — the same
`bindings = "bin"` maturin build the PyPI release uses (root `pyproject.toml`) — which `bundle
deploy` builds locally and uploads to workspace files itself, so no Volume and no hand-written
fetch step are needed for the binary itself. This is a placeholder for a PyPI dependency: once a
release tracks the CLI's `dev` branch, the `artifacts:` block is dropped in favour of a pinned
`smelt-sql==<version>` in the job environment's dependencies.

The wheel is built by `scripts/dbx-wheel-build.sh`, not a bare `maturin build`: a bare build
links against whatever glibc the build host ships, which Databricks serverless compute's
installer refuses if it is newer than the platform provides. The script builds against a
declared manylinux compatibility floor instead, and refuses to leave a wheel tagged above that
floor — or an unrepaired non-manylinux wheel — on disk for `bundle deploy` to upload.

Serverless compute can also land a job task on either `aarch64` or `x86_64` hardware, and which
one it gets can change between runs — there is no pinning mechanism on the platform side. So the
script builds one wheel per architecture (cross-compiling the `aarch64` wheel with `zig` from the
same `x86_64` dev host), and the job's `smelt_env.dependencies` lists both, each scoped by a
`platform_machine` environment marker so the installer picks the one matching whichever
architecture the run actually landed on.

The job's own `databricks` target authenticates with the **ambient** session — neither `host`
nor `token` is a key on it at all:

```yaml
targets:
  databricks_job:
    type: databricks
    catalog: workspace
    schema: smelt_dogfood
```

Measured against a real Free Edition serverless job task: no host-bearing `DATABRICKS_*`/`DBX_*`
environment variable is exported into the task's runtime, only a pre-established Spark Connect
channel (`SPARK_REMOTE`). The session is therefore built with no explicit host or token call on
the Databricks Connect builder at all, honouring whatever workspace context the runtime already
established (see "Credentials" above). The project itself, including its `.smelt/` run state, lives on a
Unity Catalog Volume — declared as a bundle resource (`resources/volume.yml`) rather than assumed
pre-existing — instead of the job's own ephemeral workspace-files checkout, so each scheduled run
is a genuine incremental window over the previous one rather than a fresh start. `bundle deploy`
creates the Volume; `scripts/dbx-bundle.sh seed` then copies `smelt.yml` and `models/` onto it.
Re-running `seed` is safe to repeat — it never touches `.smelt/`, the run-state ledger that makes
incremental windows possible, so a re-seed cannot silently reset a deployed project's state.

The loader task passes `--next-day` rather than a literal date: the fixture the dogfood pipeline
replays holds a fixed historical range with no relationship to the job trigger's real calendar
date, so a scheduled run advances the fixture by its own ledger (the live `_loader_days` table)
instead of trusting wall-clock time, and exits cleanly (not as a job failure) once the fixture is
exhausted. The loader's own DuckDB access prefers the `duckdb` Python module over shelling out to
a CLI binary, since a serverless Databricks Python environment installs packages but has no CLI
on `PATH`.

### Trino

A `trino` target reaches a Trino coordinator over its own HTTP statement protocol
(`POST /v1/statement`, paging through `nextUri`) from pure Rust — no Python interpreter, no
venv, and no third-party client library in the path. It runs against Trino's **Iceberg**
connector only: the connector, not Trino itself, decides the write surface, and `MERGE`,
`UPDATE`, `DELETE` and `CREATE OR REPLACE TABLE` exist on Iceberg but not on Hive, which the
whole incremental/ledger story downstream of this target depends on.

```yaml
targets:
  trino_prod:
    type: trino
    host: trino.example.com
    port: 8443
    user: smelt
    catalog: iceberg
    schema: analytics
    tls: true
    password: ${SMELT_TRINO_PASSWORD}
```

| Field | Required | Description |
|---|---|---|
| `type` | Yes | Must be `trino`. |
| `host` | Yes | Coordinator hostname. Must be a **bare hostname** — no scheme, no trailing slash (e.g. `trino.example.com`, not `https://trino.example.com/`). Trino has no ambient form: `host`, `catalog` and `user` are required unconditionally. |
| `port` | No | Coordinator port. Defaults to `8080` when `tls` is unset or `false`, `443` once `tls: true`. |
| `user` | Yes | Trino session user, sent as the `X-Trino-User` header. |
| `catalog` | Yes | The Iceberg catalog name — the connector choice, and never guessed. |
| `schema` | No | Schema holding created tables and views. Defaults to `main`. |
| `tls` | No | Selects `https` and the `443` default port. Defaults to `false` (plain `http`, the unauthenticated local tier). |
| `password` | No | A `${ENV}` reference to the session password, sent as HTTP Basic auth alongside `user`. **Must** be a `${VAR}` reference — a literal password is a hard configuration error, never a warning, because the value is a whole-coordinator credential that would otherwise sit in a checked-in file. Absent means no `Authorization` header at all — the unauthenticated local Docker tier's default. |

A `trino` target hard-errors, naming both the offending key and the backend, on any key
belonging to another backend's shape: `connect_url`, `warehouse`, `format`, `database`,
`settings`, `project`, `dataset`, `location`, and `token` (Databricks' credential key — a
`trino` target authenticates with `password` instead, so a Databricks-shaped credential left
over from a copy-paste is never silently ignored).

smelt compiles the same logical models against Trino using its own dialect
(`SqlDialect::Trino`). Loading data into a `trino` target goes through the client's own bulk
`INSERT INTO … SELECT CAST(…) FROM (VALUES …)` path, chunked at 1,000 rows per statement — Trino
carries no object-store credential of its own (`host`/`port`/`user`/`catalog`/`schema`/`tls`/
`password` only), so a staged-Parquet-into-object-storage load path is not available.

#### Credentials

The Trino backend authenticates with HTTP Basic auth built from `user` and a `${ENV}`-supplied
`password` — never a literal value in `smelt.yml`. `password` absent selects the unauthenticated
form the local Docker tier runs with.

```bash
export SMELT_TRINO_PASSWORD="$(cat /path/to/minted/password)"
smelt run --target trino_prod
```

The resolved password never reaches a log line, a run report, a diagnostic, or an error
message — every rendering path for a `trino` target config redacts the field.

#### Running the local tier

The Trino target is developed against a pinned `docker compose` tier: a Trino coordinator, an
Iceberg REST catalog, and MinIO object storage.

```bash
bash scripts/trino-up.sh      # coordinator on :18080 (SMELT_TRINO_PORT to override);
                               # Iceberg REST + MinIO are internal-only
source scripts/trino-env.sh   # export SMELT_TRINO_URL + catalog/schema/user
bash scripts/trino-down.sh    # remove all three containers and their named volumes
```

See `scripts/README-trino.md` for the pinned image versions and why the tier uses named
volumes rather than bind mounts. Trino-targeted tests are gated on `SMELT_TRINO_URL`: unset,
they skip; set, they run against the live tier — the same pattern as Spark's, BigQuery's and
Databricks' integration tests. CI runs the same tier in the gated `trino-integration` job.

#### Limitations

Trino's capability profile was established by executing the statement each flag names against
a live coordinator (`docs/specs/multi_backend.md` §"Capability matrix"), not by reading its
documentation. Measured `✗`s:

- `supports_qualify` — no `QUALIFY` clause; a smelt model using it is lowered to a subquery.
- `supports_double_colon_cast` — no `x::T` cast syntax.
- `supports_trailing_commas` — a trailing comma before `)` or `FROM` is a syntax error.
- `supports_pipe_syntax` / `supports_pipe_set_drop_rename` — no native `|>` pipe query support.
- `supports_transactional_ddl` — smelt's stateless `/v1/statement` HTTP client has no session
  continuity across `START TRANSACTION`/DDL/`ROLLBACK`; this measures the client, not a Trino
  grammar rejection.
- `supports_merge_not_matched_by_source` — no `WHEN NOT MATCHED BY SOURCE` merge clause.
- `supports_merge_schema_write` — no implicit schema-widening write through a `MERGE`.
- `supports_insert_overwrite` — no native `INSERT OVERWRITE`; emulated like DuckDB's and
  BigQuery's.
- `supports_native_ivm` / `supports_retraction` — no native incremental-view-maintenance or
  retraction support; smelt's own maintenance layer does not run on Trino yet (below).
- `supports_alter_column_using` — no `ALTER COLUMN ... USING` type-changing syntax.
- `supports_fingerprint_sidecar` — no delta-restriction admission over an external
  `mutable_snapshot` source's fingerprint diff.
- `staged_relation_group_is_atomic` — no transactional write capability at all, not even
  single-statement DDL inside an explicit transaction, so the staged-candidate conditional
  DELETE+INSERT's staged relation is a real, explicitly-named, explicitly-dropped table in the
  target's own schema (`staged_relation_residence = TargetSchema`) rather than a session temp
  table, reclaimed with a leading `DROP ... IF EXISTS` before every run.

Two gaps beyond the capability matrix: **no incremental/maintenance family runs on a `trino`
target yet** — `maintenance_dialect` refuses `SqlDialect::Trino`, so a full refresh is the only
route today — and **a model projecting an `array(...)` column does not decode from Trino to
Arrow yet**. Both are tracked in `docs/specs/multi_backend.md` §Known Divergences.

None of this touches `.smelt/`: single-writer locking and the `meta.json` layout-version check
are backend-independent and work normally against a `trino` target, the same as against every
other backend.

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
| `Additive`-combiner keyed/composed folds (e.g. `SUM` across a keyed or composed cumulative fold) | No Spark ledger dialect yet for the never-fold-twice reconciliation ledger; the cell takes a recorded, explain-visible downgrade (`MaintenanceStateDowngraded`) to its recompute-family equivalent rather than failing loud |
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

Where a backend's native return type for an expression differs from smelt's inferred type, output columns are reconciled to the inferred type with a `CAST`, so a model writes the same schema — same column names, same types — to every warehouse regardless of engine. Column names follow the rule in [Output column names](../reference/language.md#output-column-names): an explicit alias or a bare column reference keeps its own name; anything else (a function call, an expression, a literal) gets a synthesized, dialect-invariant `_smelt_col{n}` name.

!!! note
    Not all SQL features are available on all backends. If you use a backend-specific function, smelt will report an error when targeting a backend that does not support it.

### Position-dependent aggregate support

A backend's support for a built-in aggregate can differ by *where* it's called. smelt tracks
support separately for four call positions:

- **Scalar** — a row-wise expression, not an aggregate at all.
- **Aggregate** — the call itself is an aggregate, with `GROUP BY` and no `OVER` clause.
- **Whole-partition window** — `OVER (PARTITION BY g)` with no `ORDER BY` and no frame (or an
  explicit `BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` frame), so every row in a
  partition sees the same value.
- **Running window** — any narrower frame, including the common `OVER (PARTITION BY g ORDER BY
  t)` with no explicit frame, where the value can differ from row to row within a partition.

Some aggregates are offered by a target backend in only one of these shapes. GoogleSQL's
`PERCENTILE_CONT`/`PERCENTILE_DISC`, for example, require an `OVER` clause and cannot appear
under a `GROUP BY` at all; DuckDB and Spark have the reverse gap — `PERCENTILE_CONT`/
`PERCENTILE_DISC` are ordered-set aggregates there with no window form. `MAX_BY`/`MIN_BY` and
`APPROX_COUNT_DISTINCT` are aggregate-only on GoogleSQL, with no analytic form at all.

**A whole-partition window over an aggregate-only built-in — or an aggregate over a
window-only built-in — lowers transparently.** smelt restructures the statement around a
synthesised CTE: the source is bound once, grouped by the partition (or `GROUP BY`) keys, and the
per-partition value is joined back onto every row. Output column names and types are unchanged.
For example, on DuckDB and Spark a whole-partition `PERCENTILE_CONT` window restructures into a
grouped CTE joined back to the source:

```sql
-- as written
SELECT
    id,
    g,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med
FROM tbl
```

and on GoogleSQL, the reverse shape — an ordered-set aggregate under `GROUP BY` — restructures
into an analytic CTE read back with `ANY_VALUE`:

```sql
-- as written
SELECT g, COUNT(*) AS n, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med
FROM tbl GROUP BY g
```

Both shapes above are ordinary smelt SQL — no target-specific rewriting is needed in the model
itself.

**A whole-partition window is required.** The lowering computes one value per partition and joins
it back, so it is correct only when every row in a partition is meant to see the same value. A
**running** window over a built-in with no analytic form on the target backend has no correct CTE
form — a per-row correlated subquery would be a different construct with a different cost profile
— and is refused at compile time with `UnsupportedOnBackend`, naming the built-in, the backend,
and the whole-partition requirement (see [Diagnostics reference:
UnsupportedOnBackend](../reference/diagnostics.md#example-unsupportedonbackend)).

If your window genuinely must be running — the value legitimately differs row to row within a
partition, such as a running median as of each row's own timestamp — smelt will not synthesize
that for you, because a correct per-row form is a materially more expensive query than the
whole-partition case. Write the per-row form yourself, for example as a correlated subquery that
bounds the aggregate to the rows up to and including the current one:

```sql
SELECT
    id,
    g,
    t,
    (
        SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY inner_.x)
        FROM tbl AS inner_
        WHERE inner_.g = outer_.g AND inner_.t <= outer_.t
    ) AS running_med
FROM tbl AS outer_
```

This is the same construct smelt refuses to generate automatically, spelled out explicitly so its
cost is visible in the model's own SQL rather than hidden behind an `OVER` clause.

A restructure is also refused — with a diagnostic naming the specific rule — when the query block
around the affected call isn't a shape the restructure can rewrite in place: `ROLLUP`/`CUBE`/
`GROUPING SETS` grouping, an occurrence in `HAVING`, the query's `ORDER BY`, or `QUALIFY`, a
`DISTINCT` argument or `FILTER (WHERE …)` clause, an unexpanded `SELECT *`, a non-deterministic
`PARTITION BY` expression, or a correlated subquery. Each of these needs the same kind of manual
rewrite: pull the affected aggregate into its own `GROUP BY`/join or correlated-subquery form
before joining it back into the original query shape.

### Per-operand-type lowering

Some built-ins lower differently depending on the type of the values passed to them, not just on
where they're called. `a // b` is DuckDB's native floor/true division operator; on Spark it lowers
to `a DIV b` when both operands are integral and to plain `a / b` when both are floating-point or
decimal. When an operand's type cannot be resolved at compile time, smelt refuses with
`UnsupportedOnBackend` rather than guess — a wrong guess here would silently compute a different
number, not fail loudly. See [Diagnostics reference: a verdict that depends on operand
type](../reference/diagnostics.md#a-verdict-that-depends-on-operand-type).

### Clauses a dialect doesn't have

Two clauses smelt's SQL accepts are absent from GoogleSQL, and neither belongs to any one
function, so neither can be lowered by a per-built-in rule. Both are refused at compile time on
the `bigquery` target — naming the construct, the backend, and the rewrite — rather than being
sent to the warehouse to fail there:

| You wrote | On BigQuery | Write instead |
|---|---|---|
| `MAX(x) FILTER (WHERE p)` | GoogleSQL has no aggregate `FILTER` clause | `MAX(CASE WHEN p THEN x END)`, or `COUNT(CASE WHEN p THEN 1 END)` for a `COUNT(*)` |
| `RANGE BETWEEN INTERVAL '2 days' PRECEDING` | GoogleSQL's `RANGE` frames take a numeric offset over a numeric `ORDER BY` | `ORDER BY UNIX_MICROS(ts) RANGE BETWEEN 172800000000 PRECEDING`, or a `ROWS` frame |

Neither is rewritten for you. The `FILTER` rewrite is exactly equivalent only for aggregates that
ignore NULLs (`MIN`, `MAX`, `SUM`, `AVG`, `COUNT`, `STRING_AGG`) and would change the answer for
`ARRAY_AGG`, so smelt tells you the rewrite rather than picking one that is wrong for some
aggregates. Both clauses keep working unchanged on DuckDB and Spark.

A construct declared inside a `smelt.define` function body is refused the same way, naming the
built-in that carries it — writing it in a function is not a way around the check.

## Further reading

- [Materializations](materializations.md) for how tables and views are created in each target
- [Incremental Models](incremental-models.md) for time-partitioned processing across backends
