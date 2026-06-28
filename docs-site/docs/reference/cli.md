# CLI Commands Reference

The `smelt` command-line interface provides commands for running models, inspecting schemas, managing incremental state, and more.

## Top-Level Flags

These flags are accepted at the root command and work without a project (no `smelt.yml` required):

| Flag | Short | Description |
|------|-------|-------------|
| `--help` | `-h` | Print usage and exit. Per-subcommand `--help` prints that subcommand's flag table. |
| `--version` | `-V` | Print the installed package version (`smelt 0.x.y`) and exit. |

`smelt --version` and `smelt --help` are the only invocations that succeed without a subcommand.

## Common Flags

The following flags appear on most subcommands:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to the smelt project root (directory containing `smelt.yml`) |
| `--database` | path | _(from smelt.yml)_ | Override the DuckDB database file path |
| `--target` | string | `dev` | Target environment name as defined in `smelt.yml` |
| `--scope` | string | _(cwd-derived)_ | Dot-path prefix for expanding bare model names. Pass `--scope ''` to disable auto-scoping. |

---

## Argument resolution and `--scope`

Every command that takes an entity identifier — a model name in `--select`, a positional model argument to `smelt type`, `smelt table`, `smelt status`, etc. — resolves it using a three-shape input rule.

### The three input shapes

| Shape | Example | What happens |
|-------|---------|--------------|
| **Full path** | `silver.events_parsed` | Resolved as-is against the project. Always works. |
| **Scoped shorthand** | `events_parsed` (with scope `silver`) | Expanded to `silver.events_parsed` and resolved. Falls back to the bare argument if the expanded form does not exist. |
| **No-scope bare leaf** | `events_parsed` (no scope set) | Resolved as a full path. Errors if no entity with that exact path exists, even if a same-named entity exists in a sub-namespace. |

All smelt output — model lists, type signatures, `smelt explain --json` keys, log lines — uses the full canonical dot-path (e.g. `silver.events_parsed`) regardless of how you typed the identifier. Scope changes what you type; it never changes what smelt prints.

### Scope sources (highest precedence first)

1. **`--scope <prefix>` flag.** Pass a dot-path such as `silver` or `marts.daily`. Whitespace and the literal `smelt.` prefix are rejected.
2. **Working-directory derivation (auto).** When your current directory is inside a project's scan root (`models/` by default), smelt derives the scope from the path components between the scan root and your cwd. For example, `models/silver/` auto-scopes to `silver`; `models/marts/daily/` auto-scopes to `marts.daily`. Being at or above the scan root produces no auto-scope.
3. **No scope.** The argument must be a full path. Bare leaves error unless they are themselves full paths.

`--scope ''` (empty string) forces no scope regardless of cwd. Use this in scripts and CI where the working directory should not influence resolution.

### Worked examples — `web_analytics` project

The `web_analytics` example has a `models/silver/events_parsed.sql` model. All three forms below resolve to the same entity and produce identical output:

```bash
# 1. Full path — works from anywhere inside the project
smelt type silver.events_parsed

# 2. Explicit scope flag — same result without typing the namespace
smelt --scope silver type events_parsed

# 3. Cwd auto-scope — smelt derives "silver" from the working directory
cd models/silver
smelt type events_parsed --project-dir <project-root>
```

All three print the canonical path in their output:

```
silver.events_parsed:
  (raw_events: {...})
  -> {event_id: BIGINT, device_id: INTEGER, ...}
```

### Bare-leaf error and the `did you mean` hint

Running `smelt type events_parsed` from the project root (no auto-scope, no `--scope` flag) errors:

```
Error: Model 'events_parsed' not found. Did you mean 'silver.events_parsed'?
```

When the leaf matches multiple entities (e.g. both `silver.events_parsed` and `bronze.events_parsed` exist), the error lists all candidates and suggests using `--scope` or the full path.

### Selectors and `--scope`

`--select` and `--exclude` values are expanded through the same resolution rule. A bare name like `--select events_parsed` with scope `silver` active is expanded to `--select silver.events_parsed` before the selector engine runs. Tag selectors (`tag:staging`, `tag:revenue+`, etc.) contain a `:` and are passed through unchanged — they are not entity identifiers.

---

## smelt run

Run models and materialize them in the target database. This is the primary command for executing your data pipeline.

**Usage:**

```
smelt run [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display query results after execution |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each model |
| `--dry-run` | | bool | `false` | Parse and validate without executing |
| `--event-time-start` | | string | | Start of event time range for incremental models (ISO 8601: YYYY-MM-DD). Requires `--event-time-end`. |
| `--event-time-end` | | string | | End of event time range for incremental models (exclusive, ISO 8601: YYYY-MM-DD). Requires `--event-time-start`. |
| `--start` | | string | | Alias for `--event-time-start` |
| `--end` | | string | | Alias for `--event-time-end` |
| `--select` | `-s` | string[] | | Select models to run (repeatable). Supports: `model_name`, `tag:X`, `+tag:X`, `tag:X+`, `+tag:X+` |
| `--exclude` | `-e` | string[] | | Exclude models from the run (repeatable). Same syntax as `--select`. |
| `--batch-size` | | integer | | Override batch size in days for backfill chunking |
| `--per-partition` | | bool | `false` | Force per-partition execution (one query per granularity period) |
| `--auto` | | bool | `false` | Auto mode: process only uncovered intervals since last run |
| `--allow-column-removal` | | bool | `false` | Allow column removal during schema evolution (otherwise blocked for safety) |
| `--allow-full-refresh` | | bool | `false` | Allow full table refresh when schema changes cannot be handled with ALTER TABLE (e.g., incompatible type changes, or unsupported operations on Spark+Parquet). See [Schema Evolution](../guide/schema-evolution.md). |

**Selector syntax:**

The `--select` and `--exclude` flags support graph-aware selection:

- `model_name` -- select a single model by name (subject to scope resolution; see [Argument resolution and `--scope`](#argument-resolution-and-scope))
- `silver.model_name` -- select by full canonical path (always unambiguous)
- `tag:analytics` -- select all models with the `analytics` tag
- `+model_name` -- select the model and all its upstream dependencies
- `model_name+` -- select the model and all its downstream dependents
- `+model_name+` -- select the model, its upstreams, and its downstreams

Tag selectors (`tag:...`) are not subject to scope expansion and are passed through unchanged.

**Examples:**

```bash
# Run all models in the project
smelt run

# Run with incremental time range
smelt run --start 2026-01-01 --end 2026-01-08

# Run a specific model by full canonical path
smelt run --select silver.events_parsed

# Run with scope shorthand (equivalent to the above when scope is silver)
smelt --scope silver run --select events_parsed

# Run only models with the "staging" tag, showing compiled SQL
smelt run --select tag:staging --verbose

# Dry run to validate without executing
smelt run --dry-run

# Auto mode: process only new intervals
smelt run --auto
```

---

## smelt backbuild

Rebuild a target model and all its upstream dependencies for a specified time range. Useful for backfilling historical data or repairing a specific model and everything it depends on.

`smelt backbuild` handles both incremental and cumulative aggregate models uniformly. For incremental models, it applies the DELETE+INSERT (or append/insert-overwrite) strategy over the requested window. For `refresh: cumulative` table models, it dispatches the per-partition merge loop: each partition in the window is merged into the cumulative table without discarding earlier partitions, so accumulated state from outside the requested window is preserved.

**Usage:**

```
smelt backbuild [OPTIONS] <SELECTOR> --start <DATE> --end <DATE>
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `<SELECTOR>` | yes | Target model selector (e.g., `+marts.daily_revenue`, `silver.events_parsed`). Bare names are subject to scope resolution; see [Argument resolution and `--scope`](#argument-resolution-and-scope). |

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--start` | | string | _(required)_ | Start of time range (ISO 8601: YYYY-MM-DD) |
| `--end` | | string | _(required)_ | End of time range (exclusive, ISO 8601: YYYY-MM-DD) |
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display query results after execution |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each model |
| `--dry-run` | | bool | `false` | Show what would execute without running |
| `--batch-size` | | integer | | Override batch size in days for backfill chunking |
| `--per-partition` | | bool | `false` | Force per-partition execution (one query per granularity period) |

**Examples:**

```bash
# Backbuild a model and all its upstreams for January (canonical path form)
smelt backbuild +marts.daily_revenue --start 2026-01-01 --end 2026-02-01

# Same using scope shorthand (equivalent when scope is marts)
smelt --scope marts backbuild +daily_revenue --start 2026-01-01 --end 2026-02-01

# Preview what would be executed
smelt backbuild +marts.daily_revenue --start 2026-01-01 --end 2026-02-01 --dry-run

# Backbuild with per-partition execution
smelt backbuild +marts.daily_revenue --start 2026-01-01 --end 2026-01-08 --per-partition
```

---

## smelt build

Seed the database with CSV files and then run all models. This is a convenience command that combines `smelt seed` followed by `smelt run`.

**Lifecycle.** A single `smelt build` performs these steps in order:

1. **Load `smelt.yml`** and validate the requested `--target` exists.
2. **Discover** seed CSVs, per-entity source YAMLs, SQL models, Python models, and `smelt.define` function files — all under the directories listed in `paths:`.
3. **Seed** — for each CSV, smelt's own parser reads and type-infers the file, converts the rows to typed Arrow batches, and loads them via `Backend::load_table`. Seeds are loaded sequentially in deterministic (sorted) order. Schemas are auto-created.
4. **Plan** — build the logical graph from discovered models, apply planner rules, and produce the physical execution graph. Models are executed in topological order so each model's upstreams are materialised first.
5. **Run** — for each model, materialise according to its `materialization` (`table`, `view`, `materialized_view`, or inlined for `ephemeral`). Backends that support it use `CREATE OR REPLACE TABLE` / `CREATE OR REPLACE VIEW` for atomic replacement.

**Idempotency.** `smelt build` is safe to re-run on the same database. Seeds and non-incremental models replace their existing tables/views each run; incremental models advance their interval state and append new partitions. Re-running with the same inputs converges on the same final state.

**`--show-results`.** When set, prints a small Arrow-formatted preview after each materialised model finishes (the same renderer used by DuckDB's CLI). Use it for quick correctness spot-checks during development; it is not a substitute for `smelt test`.

**`--verbose`.** For each model that the run actually executes, prints a `-- <model_name>` comment line followed by the compiled SQL string to **stdout** immediately before the backend executes it. Output is per executed model — models skipped because they are already up-to-date produce no extra `--verbose` output. The standard `smelt: built N model(s) in T s` summary line is still printed; `--verbose` adds output, it does not replace it. Pair with `--dry-run` on `smelt run` if you want to see compiled SQL without touching the database.

### `smelt build` flag truth-table

The flags below have surprised users in practice; the table records what each one actually does on `smelt build`.

| Flag | Status | Behaviour |
|------|--------|-----------|
| `--verbose` / `-v` | implemented | Prints `-- <model_name>` + the compiled SQL to stdout immediately before each executed model. No extra output when all models are up-to-date and skipped. |
| `--show-plan` | per-model only | Requires a positional argument naming a model file path (e.g. `smelt build --show-plan models/marts/customers.sql`). There is no project-wide `--show-plan` mode — a bare `smelt build --show-plan` errors. **Output format:** success prints an `ExpandedCall fn_id="<name>"` node with the inlined function body; an unresolved function call prints `error: Unknown function \`smelt.functions...\`` to stderr and exits non-zero. |
| `--select` / `-s` | repeatable | Supply each selector as its own `--select <value>`. Space-separated values inside a single `--select` are taken as one literal selector and will not match anything; use repetition. |
| `--exclude` / `-e` | repeatable | Same selector grammar and repetition rule as `--select`. |
| `--dry-run` | **not on `smelt build`** | Use `smelt run --dry-run` for parse-and-validate-without-executing. There is no project-wide compile-only flag on `build` today. |
| `--event-time-start` / `--event-time-end` | implemented | ISO-8601 (`2026-03-20` or `2026-03-20T00:00:00Z`). End is exclusive. Both required together for incremental execution. |

**Usage:**

```
smelt build [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display query results after execution |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each model |
| `--event-time-start` | | string | | Start of event time range for incremental models (ISO 8601: YYYY-MM-DD). Requires `--event-time-end`. |
| `--event-time-end` | | string | | End of event time range for incremental models (exclusive, ISO 8601: YYYY-MM-DD). Requires `--event-time-start`. |
| `--select` | `-s` | string[] | | Select models to run (repeatable). Same syntax as `smelt run`. |
| `--exclude` | `-e` | string[] | | Exclude models from the run (repeatable). Same syntax as `--select`. |

**Examples:**

```bash
# Seed and run everything
smelt build

# Seed and run with incremental time range
smelt build --event-time-start 2026-01-01 --event-time-end 2026-01-08

# Seed and run only selected models (canonical path form)
smelt build --select marts.daily_revenue --select marts.transactions

# Same with scope shorthand
smelt --scope marts build --select daily_revenue --select transactions
```

---

## smelt seed

Load CSV seed files into the database. Seed files are CSV files placed in the directories listed under `paths:` in `smelt.yml` (default: `["models"]`).

**Usage:**

```
smelt seed [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--database` | | path | _(from config)_ | DuckDB database file path |
| `--target` | | string | `dev` | Target environment from smelt.yml |
| `--show-results` | | bool | `false` | Display loaded data after seeding |
| `--select` | `-s` | string[] | | Select specific seeds to load (by name or schema.name) |

**Examples:**

```bash
# Load all seed files
smelt seed

# Load specific seeds
smelt seed --select customers --select products

# Load and display the data
smelt seed --show-results
```

---

## smelt test

Run model tests and report results. Tests are `smelt.test` declarations in `.sql` files, placed in a directory listed in `paths:` (typically `tests/`).

Each test defines mock input data and expected output for a model or CTE. smelt compiles the test into a standalone SQL query, executes it against an in-memory DuckDB instance, and compares the result.

**Usage:**

```
smelt test [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--select` | `-s` | string[] | | Filter tests by name (repeatable, substring match) |
| `--verbose` | `-v` | bool | `false` | Show compiled SQL for each test |
| `--show-all` | | bool | `false` | Show passing tests in output (default: only failures shown) |

**Output:**

Test results are printed as PASS/FAIL lines with timing. A summary line shows total counts. The command exits with code 1 if any test fails.

```
smelt test

  PASS test_cohort_sizes (mart_cohort_retention::cohort_sizes)     0.02s
  FAIL test_user_activity (user_activity)                          0.03s

  1 passed, 1 failed, 2 total (0.05s)
```

**Examples:**

```bash
# Run all tests
smelt test

# Run tests matching "cohort"
smelt test --select cohort

# Run with compiled SQL output
smelt test --verbose

# Show all results including passes
smelt test --show-all
```

See the [Testing guide](../guide/testing.md) for how to write tests.

---

## smelt diff

Show pending schema changes between model definitions and deployed state. Compares the inferred schema (from SQL parsing and type inference) against the last deployed schema (stored in `.smelt/schemas/`).

This command does **not** require a database connection — it works entirely offline, making it fast and suitable for CI pipelines.

**Usage:**

```
smelt diff [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--select` | `-s` | string[] | | Select models to diff (repeatable). Same selector syntax as `smelt run`. |
| `--exclude` | `-e` | string[] | | Exclude models from diff (repeatable). Same syntax as `--select`. |
| `--json` | | bool | `false` | Output as JSON for machine consumption |

**Exit codes:**

- `0` — no schema changes detected
- `1` — schema changes detected (or new/removed models found)

**Output:**

For each model with changes, smelt shows the specific column-level changes and a risk assessment:

```
smelt diff

Model: daily_revenue
  ADD COLUMN email VARCHAR NULL
  ALTER COLUMN amount TYPE INTEGER -> BIGINT
  -> Safe: ALTER TABLE (no data loss)

Model: user_sessions
  DROP COLUMN legacy_id
  ADD COLUMN session_type VARCHAR NOT NULL
  -> Requires: --full-refresh (NOT NULL column added)

Model: new_model
  + New model (not yet deployed)

Summary: 2 changed, 1 new, 0 removed, 5 unchanged
```

Change types detected:

- **ADD COLUMN** — column exists in model SQL but not in deployed schema
- **DROP COLUMN** — column exists in deployed schema but not in model SQL
- **ALTER COLUMN TYPE** — column type changed (e.g., INTEGER → BIGINT)
- **ALTER COLUMN nullability** — column changed between NULL and NOT NULL

Risk assessment:

- **Safe: ALTER TABLE** — changes can be applied with ALTER TABLE statements (no data loss)
- **Requires: --full-refresh** — destructive changes that need a full table rebuild (e.g., adding NOT NULL column, unsafe type narrowing)
- **Requires: --allow-column-removal** — column removals detected (blocked by default for safety)

**Examples:**

```bash
# Show all schema changes
smelt diff

# Show changes for a specific model (canonical path form)
smelt diff --select marts.daily_revenue

# Show changes using scope shorthand
smelt --scope marts diff --select daily_revenue

# Show changes for all models with a tag
smelt diff --select tag:staging

# JSON output for CI
smelt diff --json

# Use in CI: fail if any schema changes pending
smelt diff --json || echo "Schema changes detected!"
```

**JSON output format:**

```json
{
  "models": [
    {
      "name": "daily_revenue",
      "status": "changed",
      "changes": [
        { "type": "add_column", "column": "email", "data_type": "VARCHAR", "nullable": true }
      ],
      "risk": {
        "requires_full_refresh": false,
        "has_column_removals": false,
        "migration_action": "alter_table",
        "statements": ["ALTER TABLE main.daily_revenue ADD COLUMN email VARCHAR"]
      }
    },
    { "name": "new_model", "status": "new" }
  ],
  "summary": { "changed": 1, "new": 1, "removed": 0, "unchanged": 5 }
}
```

---

## smelt docs generate

Generate a static data catalog from your project's model metadata. Exports model schemas, column lineage, descriptions, tags, and dependency information as browsable documentation.

**Usage:**

```
smelt docs generate [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--format` | | string | `markdown` | Output format: `markdown` or `json` |
| `--output` | `-o` | path | `target/docs` | Output directory |
| `--select` | `-s` | string[] | | Select models to include (repeatable). Same selector syntax as `smelt run`. |

**Output formats:**

**Markdown** (default) generates a directory with:

- `index.md` — project overview with model table and tag index
- `models/<name>.md` — per-model page with columns, lineage, and configuration

**JSON** generates a single `catalog.json` with all metadata in a structured format.

**What's included per model:**

- Name, description, owner, tags, materialization
- Columns with inferred types, nullability, descriptions, and column-level tests
- Column lineage (source tracking: from which upstream model or external table)
- Upstream and downstream dependencies
- Incremental configuration (if applicable)

**Examples:**

```bash
# Generate markdown docs
smelt docs generate

# Generate JSON catalog
smelt docs generate --format json

# Generate docs for specific models
smelt docs generate --select tag:marts

# Custom output directory
smelt docs generate --output docs/catalog
```

---

## smelt docs list

List the user-facing documentation topics shipped with this binary. Documentation is embedded at build time, so it works offline and is pinned to the installed version of smelt.

**Usage:**

```
smelt docs list
```

Each line is a topic path you can pass to `smelt docs show`.

**Example:**

```bash
smelt docs list
# concepts/how-it-works
# concepts/project-structure
# getting-started/installation
# getting-started/quickstart
# guide/incremental-models
# ...
```

---

## smelt docs show

Print the markdown contents of a documentation topic to stdout.

**Usage:**

```
smelt docs show <topic>
```

The `<topic>` argument is a path from `smelt docs list`, with or without the `.md` suffix.

**Examples:**

```bash
smelt docs show getting-started/quickstart
smelt docs show guide/incremental-models
smelt docs show reference/smelt-yml | less
```

If the topic isn't found, the error message lists near matches.

---

## smelt docs path

Explain where the embedded docs live (they are inside the binary itself, not on disk). Useful when you're scripting around the docs and want to know whether to grep through `smelt docs show` output or look for files on the filesystem.

**Usage:**

```
smelt docs path
```

---

## smelt table

Show column names and types for a model. The schema is inferred by the smelt type checker without executing the model.

**Usage:**

```
smelt table [OPTIONS] <MODEL_NAME>
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `<MODEL_NAME>` | yes | Name of the model to inspect |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--format` | string | `table` | Output format: `table` (human-readable) or `json` |

**Type caveat:** The types shown by `smelt table` are smelt's **compile-time inferred types** — they are derived from the SQL source without executing the model. They may differ from the physical column types that DuckDB stores after a `smelt build`. To see the physical DuckDB column types after building, use:

```bash
duckdb my-project.duckdb -c 'DESCRIBE <model>'
```

**Typed functions:** For columns whose values come from typed `smelt.define` calls — functions annotated with a `-> Expr<T>` return type — `smelt table` correctly reflects the declared return type. For example, a column fed by a `-> Expr<Double>` function shows as `DOUBLE` in `smelt table` output, and downstream aggregates (such as `SUM`) also use that declared type for their inferred result.

**Examples:**

```bash
# Show column types for a model (canonical path form)
smelt table marts.daily_revenue

# Using scope shorthand
smelt --scope marts table daily_revenue

# Output as JSON
smelt table marts.daily_revenue --format json

# Inspect a model in a different project
smelt table silver.users --project-dir ./my-project
```

---

## smelt type

Show the function type signature of models, displaying their input references and output columns. When called without a model name, shows signatures for all models in the project.

**Usage:**

```
smelt type [OPTIONS] [MODEL_NAME]
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `[MODEL_NAME]` | no | Name of a specific model to inspect (omit to show all) |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |

**Examples:**

```bash
# Show type signatures of all models
smelt type

# Show type signature of a specific model (canonical path form)
smelt type silver.events_parsed

# Using scope shorthand
smelt --scope silver type events_parsed

# All output uses canonical paths regardless of how you typed the name:
# silver.events_parsed:
#   (raw_events: {...})
#   -> {event_id: BIGINT, device_id: INTEGER, ...}
```

---

## smelt status

Show interval coverage and gaps for incremental models. Reports which time intervals have been materialized and identifies any gaps in coverage.

**Usage:**

```
smelt status [OPTIONS] [MODEL_NAME]
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `[MODEL_NAME]` | no | Specific model to show status for (omit for all incremental models) |

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--since` | string | | Start of query range for gap detection (ISO 8601: YYYY-MM-DD) |
| `--until` | string | | End of query range for gap detection (ISO 8601: YYYY-MM-DD, default: today) |

**Examples:**

```bash
# Show status of all incremental models
smelt status

# Show status for a specific model (canonical path form)
smelt status silver.sessions

# Using scope shorthand
smelt --scope silver status sessions

# Check for gaps in a specific time range
smelt status silver.sessions --since 2026-01-01 --until 2026-03-01
```

---

## smelt history

Show run history for the project. Displays past execution records including timestamps, durations, and which models were run.

**Usage:**

```
smelt history [OPTIONS] [MODEL_NAME]
```

**Arguments:**

| Argument | Required | Description |
|----------|----------|-------------|
| `[MODEL_NAME]` | no | Specific model to show history for (omit for all runs) |

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--limit` | `-l` | integer | `10` | Number of runs to show |

**Examples:**

```bash
# Show recent run history
smelt history

# Show last 20 runs
smelt history --limit 20

# Show history for a specific model (canonical path form)
smelt history silver.sessions
```

---

## smelt explain

Output model graph and configuration as JSON for orchestrator integration. Produces a machine-readable representation of the project's dependency graph, model configurations, and physical execution plan.

**Usage:**

```
smelt explain [OPTIONS]
```

**Flags:**

| Flag | Short | Type | Default | Description |
|------|-------|------|---------|-------------|
| `--project-dir` | | path | `.` | Path to smelt project root |
| `--json` | | bool | `false` | Output as JSON (required for machine consumption) |
| `--select` | `-s` | string[] | | Select models to include (repeatable). Same selector syntax as `smelt run`. |

The output includes both the **logical graph** (models as written) and the **physical graph** (execution plan with ephemeral models inlined, strategies resolved). See [Two-Graph Architecture](../developing/architecture.md#two-graph-architecture) for details.

**Examples:**

```bash
# Show the explain output
smelt explain

# Output as JSON for scripting
smelt explain --json

# Explain only selected models and their dependencies (canonical path form)
smelt explain --select +marts.daily_revenue --json

# Using scope shorthand
smelt --scope marts explain --select +daily_revenue --json
```

---

## smelt ui

Start a local web UI for visualizing the model graph and project structure.

**Usage:**

```
smelt ui [OPTIONS]
```

**Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--project-dir` | path | `.` | Path to smelt project root |
| `--port` | integer | `3000` | Port to serve the UI on |
| `--host` | string | `127.0.0.1` | Host address to bind to |

**Examples:**

```bash
# Start the web UI on default port
smelt ui

# Start on a custom port
smelt ui --port 8080

# Bind to all interfaces (for remote access)
smelt ui --host 0.0.0.0 --port 3000
```
