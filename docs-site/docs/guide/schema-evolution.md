# Schema Evolution

Schema evolution lets smelt automatically migrate incremental tables when your model's output schema changes. Instead of dropping and recreating a table from scratch, smelt compares the deployed schema against the new inferred schema and generates the minimal set of DDL statements to bring the table up to date.

## How it works

When an incremental model runs, smelt:

1. **Compares schemas** -- parses both the deployed and inferred column types into structured representations and diffs them recursively.
2. **Classifies changes** -- each change is categorized (column added, type widened, struct field added, etc.) and checked for safety.
3. **Plans operations** -- safe changes produce ALTER TABLE statements; unsafe changes trigger a full refresh.
4. **Executes DDL** -- the backend-specific DDL is generated and run. DuckDB, Spark+Delta, and Spark+Parquet each have their own code paths.

## Configuration

Schema evolution is configured in the model's frontmatter or in `smelt.yml`.

### Frontmatter example

```sql
---
materialization: table
schema_evolution:
  strategy: alter_and_backfill
columns:
  status:
    default: "'pending'"
  metadata:
    default: "STRUCT_PACK(version := 1, active := TRUE)"
---

SELECT
    id,
    status,
    metadata
FROM smelt.ref('upstream_model')
```

### smelt.yml example

```yaml
models:
  my_model:
    materialization: table
    schema_evolution:
      strategy: alter_and_backfill
```

### Configuration fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `schema_evolution.strategy` | string | `alter_and_backfill` | How to handle schema changes. `alter_and_backfill` uses ALTER TABLE when possible; `full_refresh` always drops and recreates. |

### Per-column fields

Per-column metadata is declared under the `columns` key:

| Field | Type | Description |
|-------|------|-------------|
| `default` | string | SQL expression used as the DEFAULT value when adding a NOT NULL column via ALTER TABLE. Examples: `"0"`, `"'unknown'"`, `"NULL"`, `"STRUCT_PACK(a := 0)"`. |
| `backfill` | string | SQL expression used in an UPDATE statement to backfill existing rows after a column is added. Example: `"CASE WHEN status IS NULL THEN 'unknown' ELSE status END"`. |

!!! note
    The `default` value is a **raw SQL expression**, not a YAML value. To set a string default, wrap it in SQL quotes: `default: "'pending'"`. For numeric defaults, just use the number: `default: "0"`.

## Safe vs unsafe changes

smelt classifies every schema change as **safe** (can be handled with ALTER TABLE) or **unsafe** (requires a full table refresh).

### Safe changes

| Change | What happens |
|--------|-------------|
| **Add a nullable column** | `ALTER TABLE ADD COLUMN` |
| **Add a NOT NULL column with default** | `ALTER TABLE ADD COLUMN ... DEFAULT expr` |
| **Widen a scalar type** (e.g., INTEGER to BIGINT) | `ALTER TABLE ALTER COLUMN TYPE` |
| **Add a nullable field to a struct** | Backend-specific DDL (see [backend matrix](#backend-capability-matrix)) |
| **Widen a type inside a struct** | `ALTER TABLE ALTER COLUMN TYPE` with the full struct type |
| **Widen array element type** (e.g., INTEGER[] to BIGINT[]) | `ALTER TABLE ALTER COLUMN TYPE` |
| **Widen map value type** | `ALTER TABLE ALTER COLUMN TYPE` |
| **Change nullability** (NOT NULL to nullable) | `ALTER TABLE ALTER COLUMN DROP NOT NULL` |

### Unsafe changes (require full refresh)

| Change | Why |
|--------|-----|
| **Remove a column** | Data loss. Allowed with `--allow-column-removal`. |
| **Narrow a type** (e.g., BIGINT to INTEGER) | Data truncation. |
| **Change a map's key type** | No safe migration path. |
| **Reorder struct fields** | Positional storage mismatch. |
| **Change between incompatible types** (e.g., struct to scalar) | No meaningful migration. |
| **Add a NOT NULL column without default** | Existing rows would violate the constraint. |

When an unsafe change is detected and the strategy is `alter_and_backfill`, smelt blocks execution and reports an error explaining the change. Use `--allow-full-refresh` to permit smelt to drop and recreate the table.

## Complex type examples

### Adding a field to a struct column

Your model previously produced:

```sql
-- v1: STRUCT(name VARCHAR, age INTEGER)
SELECT struct_pack(name := name, age := age) AS profile FROM ...
```

You add a new field:

```sql
-- v2: STRUCT(name VARCHAR, age INTEGER, email VARCHAR)
SELECT struct_pack(name := name, age := age, email := email) AS profile FROM ...
```

smelt detects the `StructFieldAdded` change and runs:

=== "DuckDB"

    ```sql
    ALTER TABLE main.my_table ADD COLUMN profile.email VARCHAR;
    ```

=== "Spark (Delta)"

    ```sql
    ALTER TABLE catalog.schema.my_table ADD COLUMNS (profile.email VARCHAR);
    ```

=== "Spark (Parquet)"

    ```sql
    ALTER TABLE catalog.schema.my_table ADD COLUMNS (profile.email VARCHAR);
    ```

Existing rows get `NULL` for the new field.

### Widening a type inside a struct

Change a field from INTEGER to BIGINT:

```sql
-- v1: STRUCT(score INTEGER, label VARCHAR)
-- v2: STRUCT(score BIGINT, label VARCHAR)
```

=== "DuckDB"

    ```sql
    ALTER TABLE main.my_table ALTER COLUMN stats TYPE STRUCT(score BIGINT, label VARCHAR);
    ```

    DuckDB handles the INTEGER to BIGINT cast automatically inside the struct.

=== "Spark (Delta)"

    Spark cannot ALTER COLUMN TYPE with USING expressions, so smelt performs a **table rewrite**:

    ```sql
    CREATE TABLE tmp_my_table AS SELECT *, CAST(stats AS STRUCT<score: BIGINT, label: STRING>) AS stats FROM my_table;
    DROP TABLE my_table;
    ALTER TABLE tmp_my_table RENAME TO my_table;
    ```

=== "Spark (Parquet)"

    Parquet files contain the original types and cannot be rewritten in place. This requires `--allow-full-refresh`.

### Adding a field to an array-of-structs

Your column is `STRUCT(id INTEGER, name VARCHAR)[]` and you add a `score` field:

```sql
-- v2: STRUCT(id INTEGER, name VARCHAR, score DOUBLE)[]
```

=== "DuckDB"

    ```sql
    ALTER TABLE main.my_table ALTER COLUMN items TYPE STRUCT(id INTEGER, name VARCHAR, score DOUBLE)[];
    ```

=== "Spark (Delta/Parquet)"

    Spark uses `mergeSchema` on write for nullable array-of-struct field additions.

### Map value evolution

Change map value type from INTEGER to BIGINT:

```sql
-- v1: MAP(VARCHAR, INTEGER)
-- v2: MAP(VARCHAR, BIGINT)
```

=== "DuckDB"

    ```sql
    ALTER TABLE main.my_table ALTER COLUMN lookup TYPE MAP(VARCHAR, BIGINT);
    ```

=== "Spark (Delta)"

    Table rewrite (Delta does not support ALTER COLUMN TYPE for maps).

=== "Spark (Parquet)"

    Requires `--allow-full-refresh`.

### Specifying defaults for complex types

Use SQL expressions for complex type defaults:

```yaml
columns:
  metadata:
    default: "STRUCT_PACK(status := 'unknown', count := 0)"
  tags:
    default: "[]::VARCHAR[]"
  settings:
    default: "MAP {}"
  scores:
    default: "ARRAY[1, 2, 3]"
```

### Multi-step evolution

Multiple changes to the same struct are combined into a single ALTER:

```sql
-- v1: STRUCT(a INTEGER, b VARCHAR)
-- v2: STRUCT(a BIGINT, b VARCHAR, c BOOLEAN)
```

smelt detects both the type widening (`a: INTEGER -> BIGINT`) and the field addition (`c`) and handles them in one operation.

## Backend capability matrix

Not all backends support the same schema evolution operations. The table below shows what each backend can handle natively vs. what requires a fallback.

| Operation | DuckDB | Spark + Delta | Spark + Parquet |
|-----------|--------|---------------|-----------------|
| Add nullable column | ALTER TABLE | ALTER TABLE | ALTER TABLE |
| Add NOT NULL column (with default) | ALTER TABLE | ALTER TABLE | Full refresh |
| Remove column | ALTER TABLE | ALTER TABLE (column mapping) | Full refresh |
| Widen scalar type | ALTER COLUMN TYPE | ALTER COLUMN TYPE (safe widenings) | Full refresh (most types) |
| Add struct field (nullable) | `ADD COLUMN col.field` | `ADD COLUMNS (col.field)` | `ADD COLUMNS (col.field)` (metastore) |
| Remove struct field | `DROP COLUMN col.field` | `DROP COLUMN` (column mapping) | Full refresh |
| Widen type in struct | ALTER COLUMN TYPE (full struct) | Table rewrite | Full refresh |
| Widen array element type | ALTER COLUMN TYPE | Table rewrite | Full refresh |
| Add field to array-of-structs | ALTER COLUMN TYPE (full type) | mergeSchema write | mergeSchema write |
| Widen map value type | ALTER COLUMN TYPE | Table rewrite | Full refresh |
| Change map key type | Full refresh | Full refresh | Full refresh |
| Backfill expression (UPDATE) | UPDATE statement | UPDATE statement | Full refresh |

### Recommendations

- **DuckDB** has the most complete schema evolution support. All safe changes can be handled with ALTER TABLE.
- **Spark + Delta** supports most operations. Use Delta when you need struct field removal (requires column mapping) or type widening inside nested types (via table rewrite).
- **Spark + Parquet** is the most limited. Consider switching to Delta format if you need frequent schema changes. Many operations that are safe on Delta require `--allow-full-refresh` on Parquet.

### Table format configuration

Spark targets default to Delta format. You can override per-target or per-model:

```yaml
# Per-target
targets:
  spark_prod:
    type: spark
    connect_url: sc://localhost:15002
    schema: main
    format: delta  # or "parquet"

# Per-model override
models:
  legacy_table:
    format: parquet
```

See [Project Configuration](../reference/smelt-yml.md) for full details.

## The `--allow-full-refresh` flag

When smelt detects a schema change that cannot be handled with ALTER TABLE, it blocks execution by default:

```
Error: Schema evolution requires full refresh for model 'my_model':
  Parquet format does not support nested type widening.
  Consider using Delta format or run with --allow-full-refresh.
```

Pass `--allow-full-refresh` to permit smelt to drop and recreate the table:

```bash
smelt run --allow-full-refresh
```

!!! warning
    Full refresh reprocesses the entire table from scratch. For large tables, this can be expensive. The flag exists to make this an intentional choice rather than a silent surprise.

## Further reading

- [Incremental Models](incremental-models.md) for how incremental processing works
- [Targets & Backends](targets.md) for backend configuration
- [Project Configuration](../reference/smelt-yml.md) for `smelt.yml` reference
