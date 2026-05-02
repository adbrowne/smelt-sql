---
feature: project_config
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Project Configuration

> **What this is.** A normative spec for `smelt.yml` — the project configuration file. Covers project identity, target declarations, per-model overrides, and project-level defaults.

## Surface

### smelt.yml location

`smelt.yml` must be at the project root (the directory passed to `--project-dir`, or the current directory by default). smelt does not search parent directories.

### Top-level fields

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | required | Project name. Used in run logs and catalog output. |
| `version` | integer | `1` | Config schema version. Currently decorative — the only valid value is `1`. Optional; defaults to `1` if omitted. |
| `model_paths` | string[] | `["models"]` | Directories scanned recursively for `.sql` and `.py` model files. Resolved relative to project root. |
| `seed_paths` | string[] | `["seeds"]` | Directories scanned for CSV seed files. Resolved relative to project root. |
| `targets` | map | required | Named execution environments. Must contain at least one entry. See Target fields. |
| `default_materialization` | string | `"view"` | Materialization applied to models that set no materialization anywhere. Valid values: `table`, `view`, `ephemeral`, `materialized_view`. |
| `models` | map | `{}` | Per-model configuration keyed by model name. See Model config fields. |
| `python` | string | — | Path to the Python interpreter for Python models. Overridden by the `SMELT_PYTHON` environment variable; the env var takes precedence. |
| `unstable_schema` | bool | `false` | Enable unstable schema evolution features. Parsed separately from the main config struct; the value is read by text scan, not YAML deserialization. |

### Target fields

Under `targets:`, each key is a target name (arbitrary string). The target named `dev` is the CLI default when `--target` is not specified.

**DuckDB target:**

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb   # path to .duckdb file; created if absent
    schema: main                  # schema for created tables/views
```

| Key | Type | Description |
|-----|------|-------------|
| `type` | `"duckdb"` | Required. Identifies the backend. |
| `database` | string | Path to the DuckDB database file, relative to project root. Created automatically if absent. |
| `schema` | string | Required. Default schema for all DDL. |

**Spark target:**

```yaml
targets:
  spark_prod:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog         # optional Spark catalog name
    schema: main
    warehouse: /data/warehouse     # base directory for Parquet/Delta files
    format: delta                  # default; or "parquet"
```

| Key | Type | Description |
|-----|------|-------------|
| `type` | `"spark"` | Required. Identifies the backend. |
| `connect_url` | string | Required. Spark Connect URL (e.g., `sc://host:port`). |
| `catalog` | string | Optional. Spark catalog name; defaults to the cluster's default catalog. |
| `schema` | string | Required. Default schema for all DDL. |
| `warehouse` | string | Base directory for file-based output (Parquet/Delta). Used for cross-engine data exchange. |
| `format` | `"delta"` \| `"parquet"` | Table format for Spark models. Defaults to `delta`. Affects schema evolution capabilities. |

Unknown target types fall back to DuckDB (backwards-compatibility behavior; not a supported pattern).

### Target selection precedence (per-model)

1. `target:` in the model's YAML frontmatter
2. `models.<name>.target` in `smelt.yml`
3. `--target` CLI flag (default: `dev`)

`ephemeral` and `test` models cannot have a target override. Any other model may pin its target to a specific named target regardless of the `--target` flag.

### Model config fields (`models:<name>:`)

```yaml
models:
  daily_revenue:
    materialization: table
    target: spark_prod
    tags: [revenue, daily]
    incremental:
      enabled: true
      event_time_column: order_time
      partition_column: order_date
      granularity: day
    schema_evolution:
      strategy: alter_and_backfill
    columns:
      status:
        default: "'pending'"
        backfill: "CASE WHEN status IS NULL THEN 'pending' ELSE status END"
        description: Order status
        tests: [not_null, {accepted_values: [pending, shipped, delivered]}]
    format: delta
```

| Key | Type | Description |
|-----|------|-------------|
| `materialization` | enum | Materialization type; overrides `default_materialization`. |
| `target` | string | Override execution target for this model. |
| `tags` | string[] | Tags merged (union) with frontmatter tags. |
| `incremental` | object | Incremental configuration. See `incremental_models.md`. |
| `schema_evolution` | object | Schema evolution strategy. See `schema_evolution.md`. |
| `columns` | map | Per-column metadata (`default`, `backfill`, `description`, `tests`). |
| `format` | `"delta"` \| `"parquet"` | Table format override. Only effective on Spark targets. |

### Cross-engine data exchange

When a model on DuckDB references a model pinned to Spark (via `smelt.models.<name>`), smelt resolves the reference to a `read_parquet()` call against the Spark model's Parquet files in the `warehouse` directory. No explicit copy step is needed; DuckDB reads the files natively.

This requires the Spark model to have `materialization: table` and the Spark target to have a `warehouse` path configured.

## Semantics

### Target default

The CLI flag `--target` defaults to `dev` on all commands (`run`, `build`, `seed`, `test`, `backbuild`, `table`, `type`, `status`, `history`, `docs`). If no target named `dev` exists in `smelt.yml` and no `--target` is specified, smelt fails with a "target not found" error.

### Config loading

smelt loads `smelt.yml` with `serde_yaml`. An unknown top-level key does **not** cause an error (the config struct does not use `deny_unknown_fields`). This differs from model frontmatter, which does reject unknown keys.

`unstable_schema` is a special case: it is parsed by a text scan (`parse_unstable_schema_flag`) rather than through serde deserialization, so it can be set in the file alongside the regular config without causing a YAML parse error or unknown-field warning.

### `model_paths` and `seed_paths`

Paths are resolved relative to the directory that contains `smelt.yml`. The directories must exist; paths that do not exist are silently skipped (not an error). Models nested arbitrarily deep within `model_paths` directories are discovered.

### Python interpreter resolution

1. `SMELT_PYTHON` environment variable (overrides all)
2. `python:` field in `smelt.yml`
3. Platform default (`python3` on Unix, `python` on Windows)

## Design

**Target-named `dev` as default.** Rather than requiring an explicit default target declaration, smelt uses the convention that the target named `dev` is the default. This mirrors common practice (development targets are almost always named `dev`) and avoids a confusing `default_target:` field that just points to another key in the same map.

**`version` is decorative.** The field exists so users can signal intent (`version: 1`) and so smelt can reject clearly incompatible config files if the schema ever changes dramatically. Today it has no validation effect — the field is accepted but not checked. Making it optional in the code (via `serde(default)`) removes a common new-user trip-hazard where a semver string (`version: "0.1.0"`) causes a confusing parse error.

**No `deny_unknown_fields` on Config.** Model frontmatter uses strict validation because typos there silently drop configuration. `smelt.yml` is a project-level file that changes infrequently and is usually reviewed; lenient parsing lets forward-compatible configs work across smelt versions without failing. The trade-off is that typos in `smelt.yml` keys are silently ignored.

**`unstable_schema` as a text-scanned flag.** The flag enables schema evolution features that may change behavior in breaking ways. Parsing it outside of serde means it can be set without changing the main deserialization path, and the absence of the key cleanly defaults to `false`.

**Cross-engine via Parquet, not copy.** DuckDB reading Spark Parquet files directly (via `read_parquet()`) avoids a copy step. smelt resolves the cross-engine reference at compile time; the generated SQL contains a literal `read_parquet(...)` path, not a smelt reference. See `targets.md` (user docs) and the feedback record in project memory for rationale.

## Constraints & Invariants

1. **`smelt.yml` must be at the project root.** No parent-directory search; no implicit config discovery from nested directories.
2. **`targets` must be non-empty.** A project with no targets cannot build anything; smelt errors on load.
3. **Default target is always `dev`.** CLI `--target` defaults to `dev` across all commands. If no `dev` target exists and no `--target` is specified, execution fails.
4. **`SMELT_PYTHON` overrides `python:`.** Environment variable always wins; this allows CI and local environments to differ without editing `smelt.yml`.
5. **Model config keys in `smelt.yml` must match model names exactly.** The key under `models:` is compared by exact string equality to the model name derived from the file stem. No glob or wildcard matching.

## Known Divergences / Open Questions

- **`version` documented as required in user docs.** The user-facing `smelt-yml.md` says `version` is required, but the code has a `serde(default)` that makes it optional. The user docs should be corrected to say "optional, defaults to 1."
- **`database` typed as optional for DuckDB in code.** The `Target.database` field is `Option<String>` in the Rust struct, but DuckDB cannot function without a database path. The code falls back to some default if absent; this is not documented. The field should be documented as effectively required for DuckDB targets.
- **`unstable_schema` undocumented in user guide.** The `smelt-yml.md` reference page does not mention `unstable_schema`. Its purpose and the specific behaviors it enables need to be documented.
- **No validation for unknown target types.** A `type: postgres` target falls back to DuckDB silently. This should be an error.
- **`warehouse` undocumented in smelt-yml.md.** The `warehouse` field on Spark targets is in the code but not in the reference page.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — `Config`, `Target`, `ModelConfig`, `TableFormat`, `parse_unstable_schema_flag()`, `get_target()`
  - `crates/smelt-cli/src/main.rs` — `--target` flag with `default_value = "dev"`
  - `crates/smelt-cli/src/logical_graph.rs` — `LogicalGraph::build()`, cross-engine ref resolution
- **Tests**:
  - `crates/smelt-core/src/config.rs` (inline `#[cfg(test)]`) — config loading, target selection, materialization override
- **User docs**:
  - `docs-site/docs/reference/smelt-yml.md`
  - `docs-site/docs/guide/targets.md`
- **Related specs**:
  - `models.md` — frontmatter keys that interact with project config
  - `incremental_models.md` — `incremental:` config block
  - `architecture.md` — `smelt.<path>` addressing and multi-engine design
