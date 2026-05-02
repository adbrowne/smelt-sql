---
feature: sources
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Sources

> **What this is.** A normative spec for `sources.yml` — the declaration of external tables not managed by smelt. Covers the file format, column type declarations, addressing, and the role of source declarations in type checking and LSP support.

## Surface

### File location

`sources.yml` (or `sources.yaml`) at the project root, alongside `smelt.yml`. If neither file exists, smelt loads an empty source list (no error). `sources.yml` takes precedence if both exist.

### File format

```yaml
version: 1          # required; only valid value is 1

sources:
  <schema_name>:
    description: "optional schema description"
    database: "optional database override"
    schema: "optional schema override"
    tables:
      <table_name>:
        identifier: "optional real table name"  # if the table's DB name differs
        description: "optional table description"
        columns:
          - name: <column_name>
            type: <SQL_TYPE>          # optional; see Supported types
            description: "optional"
            data_latency: "3 days"    # optional; for incremental safety analysis
```

### Top-level fields

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `version` | integer | yes | File format version. Only valid value: `1`. |
| `sources` | map | yes | Map from schema name to source definition. |

### Source definition fields

| Key | Type | Description |
|-----|------|-------------|
| `database` | string | Optional override for the database name. When absent, uses the target's configured database. |
| `schema` | string | Optional override for the schema name. When absent, the map key is used as the schema name. |
| `description` | string | Human-readable schema-level description. Surfaced in data catalog. |
| `tables` | map | Map from table name to table definition. |

### Table definition fields

| Key | Type | Description |
|-----|------|-------------|
| `identifier` | string | The real table name in the database, if it differs from the map key. When absent, the map key is used as the table name. |
| `description` | string | Human-readable table description. Surfaced in data catalog. |
| `columns` | list | List of column definitions (optional; see Column declarations). |

### Column definition fields

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `name` | string | yes | Column name. |
| `type` | string | no | SQL type (see Supported types). If omitted, smelt treats the column type as unknown and cannot type-check references to it. |
| `description` | string | no | Column description. Surfaced in data catalog. |
| `data_latency` | string | no | Late-arrival window for this column (e.g., `"3 days"`, `"0 hours"`). Used by incremental safety analysis. |

### Supported column types

```
INTEGER   BIGINT    SMALLINT
FLOAT     DOUBLE
DECIMAL   DECIMAL(p,s)
VARCHAR   TEXT      CHAR
BOOLEAN
DATE      TIMESTAMP
JSON
```

Type strings are case-insensitive and parsed via `smelt-types::parse_type()`. Unrecognized type strings are silently treated as unknown (no error at load time; the column becomes untyped for checking purposes).

### Reference syntax

```sql
FROM smelt.sources.<schema_name>.<table_name>
```

The `<schema_name>` and `<table_name>` must match the map keys in `sources.yml`, not the `identifier` or `schema` override values. Resolution uses the declared names, not the database names.

## Semantics

### Loading

`SourcesConfig::load()` looks for `sources.yml` first, then `sources.yaml`. If neither exists, an empty `SourcesConfig` is returned — no error. YAML parse errors are hard failures.

### Source resolution

`smelt.sources.<schema>.<table>` is resolved by:
1. Finding the source with `name == <schema>`
2. Within it, finding the table with `name == <table>`

The `database`, `schema`, and `identifier` fields on the source/table definitions affect what SQL is generated (which physical table is read), but not how the reference is written in model SQL.

### Column declarations are advisory

Column declarations serve type checking, LSP completions, and diagnostics. They are not validated against the live database at load time or at run time. If the live table has a column not declared in `sources.yml`, smelt will not flag undeclared columns as errors (it only checks that referenced columns are declared if the source has columns declared at all). If the live table is missing a declared column, the error surfaces at query execution time, not at smelt compile time.

### Type parsing failure is silent

If a `type:` value is not recognized by `smelt-types::parse_type()`, the column's `data_type` is set to `None` — the column becomes untyped for purposes of type checking. No warning or error is emitted. This is a known divergence from the strict-type philosophy elsewhere.

## Design

**Declaration over discovery.** Sources are not auto-discovered from the live database. Users declare columns explicitly because: (a) smelt can function without a live connection (offline development, CI), (b) declared types represent the *intended* schema rather than whatever the ingestion pipeline happened to produce, and (c) declaration enables offline LSP diagnostics without a live connection.

**`smelt.sources` namespace shared with subdirectory seeds.** Subdirectory seeds (e.g., `seeds/raw/file.csv`) are loaded into the `raw` schema and are addressable as `smelt.sources.raw.file`. This is the same namespace as `sources.yml` declarations. Seeds and sources share addressing because they serve the same role from a model's perspective: inputs that smelt didn't create.

**`identifier` for aliased tables.** The `identifier` field allows the logical name in smelt to differ from the physical table name. This is useful when ingestion tools create tables with auto-generated or environment-specific suffixes (e.g., `users_v2`) but models should reference a stable name.

**`data_latency` on columns.** Sources can carry late-arrival metadata per column for incremental safety analysis. A source column with `data_latency: "3 days"` signals that data for a given event time may arrive up to 3 days late, which informs the lookback calculation for incremental models that consume it.

## Constraints & Invariants

1. **`sources.yml` is at the project root.** No multi-file sources, no per-directory sources files.
2. **Sources are not materialized by smelt.** smelt never creates, modifies, or drops source tables. The user is responsible for ensuring sources exist before running models that reference them.
3. **Source reference uses declared names, not physical names.** `smelt.sources.raw.users` refers to the source declared with key `users` in schema `raw`, regardless of `identifier`.
4. **Column declarations are optional per table.** A table with no `columns:` list is valid; smelt treats all column references on it as having unknown type.

## Known Divergences / Open Questions

- **`database` and `schema` override fields undocumented in user guide.** `SourcesConfig` supports `database` and `schema` overrides at the source level, but `sources-yml.md` and `sources.md` do not mention them.
- **`identifier` field undocumented in user guide.** The `identifier` table field is in the code but not in the user-facing reference page.
- **Silent type parse failure.** Unrecognized type strings (`type: UNKNOWNTYPE`) produce no diagnostic and silently make the column untyped. Should emit a warning.
- **`data_latency` undocumented in user guide.** The column field exists in the code and is used by incremental safety analysis but is not documented in `sources-yml.md`.
- **`version` field is required per user docs but not validated.** The code does not validate that `version == 1` — any integer (or even absence of the field) is accepted without error.

## References

- **Code**:
  - `crates/smelt-core/src/sources.rs` — `SourcesConfig`, `SourceDef`, `SourceTableDef`, `SourceColumnDef`, `SourcesConfig::load()`
  - `crates/smelt-types/src/lib.rs` — `parse_type()`
- **Tests**:
  - `crates/smelt-core/src/sources.rs` (inline `#[cfg(test)]`) — data latency parsing, basic load
- **User docs**:
  - `docs-site/docs/guide/sources.md`
  - `docs-site/docs/reference/sources-yml.md`
- **Related specs**:
  - `seeds.md` — subdirectory seeds share the `smelt.sources` namespace
  - `models.md` — `smelt.sources.<schema>.<table>` reference syntax
  - `incremental_models.md` — `data_latency` used in batch safety analysis
