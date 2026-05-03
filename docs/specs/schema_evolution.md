---
feature: schema_evolution
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Schema Evolution

> **What this is.** A normative spec for smelt's schema evolution system — change classification (safe vs. unsafe), `ALTER TABLE` strategy, `smelt diff` behavior, stored schema format, and backend capability matrix.

## Surface

### `smelt diff` output

```
smelt diff [--select <selector>] [--format text|json]
```

`smelt diff` compares the inferred schema (derived from current model SQL) against the deployed schema (stored in `.smelt/schemas/`). It prints a per-model report and exits `0` if no changes are found, `1` if any change is detected.

Default text output per model:

```
model_name: CHANGED
  ADD COLUMN amount DOUBLE NULL
  ALTER COLUMN status VARCHAR → TEXT
  Migration: ALTER TABLE (2 statements)

model_name: NEW
model_name: UNCHANGED
model_name: REMOVED
```

`--format json` produces a machine-readable object:

```json
{
  "models": {
    "<model_name>": {
      "status": "new" | "unchanged" | "changed" | "removed",
      "changes": [
        {
          "type": "AddColumn" | "RemoveColumn" | "ChangeType" | "ChangeNullability"
               | "StructFieldAdded" | "StructFieldRemoved" | "NestedTypeChange"
               | "ArrayElementTypeChange" | "MapKeyTypeChange" | "MapValueTypeChange"
               | "IncompatibleTypeChange",
          "column": "<col_name>",
          "detail": "<string>"
        }
      ],
      "warnings": ["<string>"],
      "risk": {
        "requires_full_refresh": true | false,
        "has_column_removals": true | false,
        "migration_action": "NoChange" | "AlterTable" | "FullRefresh" | "FullRefreshBlocked"
                          | "RequiresColumnRemovalFlag" | "TableRewrite",
        "statements": ["<DDL string>"]
      }
    }
  },
  "summary": {
    "changed": 0, "new": 0, "removed": 0, "unchanged": 0
  }
}
```

### Evolution flags

| Flag | Command | Description |
|------|---------|-------------|
| `--allow-column-removal` | `run`, `build` | Allow `DROP COLUMN` for removed columns. Without this flag, column removals are blocked and the run halts. |
| `--allow-full-refresh` | `run`, `build` | Allow full table recreation for unsafe changes. Without this flag, unsafe changes halt the run. |

### Column evolution annotations

Model frontmatter may include per-column annotations used during evolution:

```yaml
columns:
  - name: status
    default: "'active'"        # SQL expression for new NOT NULL column default
    backfill: "lower(status)"  # SQL UPDATE expression run after column addition
```

| Key | Description |
|-----|-------------|
| `default` | SQL literal used as the DEFAULT expression when adding a NOT NULL column. Required when adding a NOT NULL column to an existing table. |
| `backfill` | SQL expression applied in an UPDATE statement after the column is added, to populate existing rows. |

## Semantics

### Stored schemas

When a model is materialized by `smelt run` or `smelt build`, smelt writes the deployed schema to `.smelt/schemas/<model_name>.json`. The stored schema contains:

- `version`: Integer, incremented by 1 on each successful migration
- `deployment_timestamp`: ISO 8601 timestamp
- `model_hash`: SHA-256 of the model SQL at deploy time
- `columns`: Array of `{name, data_type, nullable}` objects

If `.smelt/schemas/` does not exist, `smelt diff` reports all models as `new`.

`smelt diff` does not require a live database connection. It reads the stored schemas and runs type inference on the current model SQL offline.

### Change classification

The following change types are detected:

| Change | Classification |
|--------|---------------|
| Add nullable column | **Safe** — ALTER TABLE ADD COLUMN |
| Add NOT NULL column with `default:` | **Safe** — ALTER TABLE ADD with DEFAULT |
| Add NOT NULL column without `default:` | **Blocked** — requires `--allow-full-refresh` |
| Remove column | **Blocked by default** — requires `--allow-column-removal` |
| Widen scalar type (see widening table) | **Safe** — ALTER TABLE ALTER COLUMN TYPE |
| Narrow scalar type | **Blocked** — requires `--allow-full-refresh` |
| Incompatible type change (struct↔scalar) | **Blocked** — requires `--allow-full-refresh` |
| Change NOT NULL → NULL | **Safe** — ALTER TABLE ALTER COLUMN |
| Change NULL → NOT NULL | **Blocked** — requires `--allow-full-refresh` unless `default:` is set |
| Add nullable struct field | **Safe** |
| Remove struct field | **Blocked** — requires `--allow-column-removal` |
| Reorder struct fields | **Blocked** — requires `--allow-full-refresh` |
| Widen array element type | **Safe** if element widening is safe |
| Change map key type | **Always blocked** — requires `--allow-full-refresh` |
| Widen map value type | **Safe** if value widening is safe |

### Safe scalar type widenings

A type change is a safe widening if it cannot cause data loss:

| From | To |
|------|----|
| `SMALLINT` | `INTEGER`, `BIGINT` |
| `INTEGER` | `BIGINT` |
| `FLOAT` | `DOUBLE` |
| `CHAR` | `VARCHAR` |
| `VARCHAR(n)` | `VARCHAR` (unbounded) |
| `DECIMAL(p,s)` | `DECIMAL(p2,s2)` where p2 ≥ p and s2 ≥ s |
| Any type | Nullable version of same type (NOT NULL → NULL) |

Widening chains are transitive: `SMALLINT → BIGINT` is safe even though it skips `INTEGER`.

### Migration actions

After classifying changes, smelt resolves one `MigrationAction`:

| Action | Condition | Behavior |
|--------|-----------|---------|
| `NoChange` | Schema matches deployed | No DDL emitted |
| `AlterTable` | All changes are safe | Executes ALTER TABLE statements |
| `RequiresColumnRemovalFlag` | Column removal present, flag absent | Run halts with error |
| `FullRefreshBlocked` | Unsafe change, `--allow-full-refresh` absent | Run halts with error |
| `FullRefresh` | Unsafe change, `--allow-full-refresh` present | Table dropped and recreated |
| `TableRewrite` | Spark backend, complex change | CREATE TABLE AS SELECT + rename |

### Backend capability matrix

Not all ALTER TABLE operations are supported on all backends:

| Operation | DuckDB | Spark + Delta | Spark + Parquet |
|-----------|--------|--------------|-----------------|
| ADD COLUMN (nullable) | ✓ | ✓ | ✓ |
| ADD COLUMN (NOT NULL) | ✓ | ✗ | ✗ |
| DROP COLUMN | ✓ | ✓ (column mapping) | ✗ |
| ALTER COLUMN TYPE (safe widening) | ✓ | Limited | ✗ |
| ALTER COLUMN NULLABILITY | ✓ | ✗ | ✗ |
| Struct field addition | ✓ | ✓ (mergeSchema) | ✗ |
| Struct field removal | ✓ | ✗ | ✗ |
| Nested type widening | ✓ | ✗ | ✗ |

For Spark + Parquet, most changes that require DDL result in `FullRefresh`. For Spark + Delta, complex changes use a `TableRewrite` strategy (CREATE TABLE AS SELECT + DROP + RENAME).

DuckDB uses `struct_pack()` expressions to rewrite struct columns during ALTER TABLE for nested type changes and struct field additions or removals.

### DuckDB ALTER TABLE for structs

When a struct column requires changes (field addition, removal, or nested type widening), DuckDB generates:

```sql
ALTER TABLE schema.table
  ALTER COLUMN meta TYPE STRUCT(id INTEGER, value VARCHAR)
  USING struct_pack(id := meta.id::INTEGER, value := meta.value::VARCHAR)
```

The `USING` clause re-packs the struct field-by-field, applying casts as needed for type widenings and omitting removed fields.

## Design

**Offline diff.** `smelt diff` does not connect to the live database. Stored schemas in `.smelt/schemas/` serve as the deployed-state snapshot. This enables CI checks without database credentials and means `smelt diff` reflects the *last deployed* schema, not the current live one. If the database was modified outside of smelt, the stored schema will diverge from reality.

**ALTER TABLE over full refresh as default.** The system prefers non-destructive ALTER TABLE operations for additive changes. Full table recreation is reserved for changes that cannot be expressed as additive DDL. This avoids unnecessary data movement on large tables.

**Column removal is always opt-in.** Dropping columns is a destructive operation that cannot be reversed. Requiring `--allow-column-removal` makes the intent explicit in pipelines and prevents accidental column drops from model renames or typos.

**`backfill:` for NOT NULL column additions.** Adding a NOT NULL column requires populating existing rows. Rather than silently inserting NULL (which would violate the constraint) or always requiring a full refresh, smelt supports `default:` (column default for new rows) and `backfill:` (UPDATE expression for existing rows) as first-class evolution primitives.

**Backend capability is the binding constraint.** Safe widenings are safe in principle, but each backend limits which DDL it can execute. The planner resolves the most capable action the backend supports, falling back to TableRewrite or FullRefresh when DDL is unavailable.

## Constraints & Invariants

1. **`smelt diff` requires no live connection.** Schema comparison is entirely offline using `.smelt/schemas/`.
2. **Column removals require `--allow-column-removal`.** Without this flag, any model run that would drop a column halts with an error.
3. **Unsafe type changes require `--allow-full-refresh`.** Without this flag, narrowing or incompatible changes halt the run.
4. **Stored schema version increments on each migration.** Version starts at 1 on first deployment and increments by 1 per successful run that changes the schema.
5. **Map key type changes are always blocked from safe ALTER.** Map key changes are never a safe widening regardless of key types.
6. **Spark + Parquet receives no ALTER TABLE DDL.** Any schema change on a Parquet-backed Spark table results in either a FullRefresh or TableRewrite.

## Known Divergences / Open Questions

- **`.smelt/schemas/` not documented in user guide.** The stored schema directory, its format, its update timing, and its lifecycle are not documented for users. Users who delete it accidentally will see all models reported as `NEW` by `smelt diff`.
- **`model_hash` not used for change detection.** The stored model SQL hash is recorded but not used by `smelt diff` to decide whether a model needs re-running — only schema column differences trigger migration actions.
- **`backfill:` and `default:` undocumented in user guide.** The column evolution annotations are implemented but absent from the schema evolution user guide page.
- **`smelt diff --format json` schema not published.** The JSON output format is not documented as a stable contract. Orchestrators consuming it could break on version changes.
- **Struct field reordering detection.** Whether changing struct field order is detected as `IncompatibleTypeChange` or silently ignored depends on the comparison implementation. Current behavior undocumented in user guide.
- **Exit code for blocked migrations.** When a run is blocked by `RequiresColumnRemovalFlag` or `FullRefreshBlocked`, the exit code is non-zero but the specific code (1 vs. other) is not specified.

## References

- **Code**:
  - `crates/smelt-state/src/schema_tracking.rs` — `SchemaChange`, `SchemaDiff`, `MigrationAction`, `is_safe_type_widening()`, `diff_schemas()`, `plan_migration_for_backend()`
  - `crates/smelt-state/src/file_store.rs` — `DeployedSchema`, `.smelt/schemas/` read/write
  - `crates/smelt-cli/src/migration.rs` — `SchemaEvolutionResult`, `check_and_migrate()`, `extract_evolution_maps()`
  - `crates/smelt-cli/src/commands/diff.rs` — `smelt diff` command, JSON output
  - `crates/smelt-backend-duckdb/src/ddl_duckdb.rs` — DuckDB ALTER TABLE generation, `build_struct_pack_expr()`
  - `crates/smelt-backend-spark/src/ddl_spark.rs` — Spark DDL generation, `classify_operation()`
- **User docs**:
  - `docs-site/docs/guide/schema-evolution.md`
- **Related specs**:
  - `models.md` — model materialization modes
  - `cli.md` — `smelt diff` command, `smelt run` flags
  - `smelt_yml.md` — target backend configuration
