# Schema Evolution

Schema evolution lets smelt automatically migrate incremental tables when your model's output schema changes. Instead of dropping and recreating a table from scratch, smelt compares the deployed schema against the new inferred schema and generates the minimal set of DDL statements to bring the table up to date.

## How it works

When an incremental model runs, smelt:

1. **Compares schemas** -- parses both the deployed and inferred column types into structured representations and diffs them recursively.
2. **Classifies changes** -- each change is categorized (column added, type widened, struct field added, etc.) and checked for safety.
3. **Plans operations** -- safe changes produce ALTER TABLE statements; unsafe changes trigger a full refresh.
4. **Executes DDL** -- the backend-specific DDL is generated and run. DuckDB, Spark+Delta, Spark+Parquet, and BigQuery each have their own code paths.

## Configuration

Schema evolution is configured in the model's **SQL frontmatter**. The `schema_evolution` and `columns` keys are frontmatter-only — they have no effect if placed under `models.<name>:` in `smelt.yml`.

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
FROM smelt.upstream_model
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
    ALTER TABLE catalog.schema.my_table ADD COLUMNS (profile.email STRING);
    ```

=== "Spark (Parquet)"

    Not expressible: a v1 Parquet table rejects a qualified path in `ADD COLUMNS`. The run is
    refused with a message naming the struct column and the field, and needs
    `--allow-full-refresh` to rebuild the model instead.

=== "BigQuery"

    Not expressible: GoogleSQL has no dotted `ADD COLUMN`, and `ALTER COLUMN … SET DATA TYPE`
    refuses a struct that gained a field. The run is refused with a message naming the struct
    column and the field, and needs `--allow-full-refresh` to rebuild the model instead.

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

    Delta adds the field in place:

    ```sql
    ALTER TABLE catalog.schema.my_table ADD COLUMNS (items.element.score DOUBLE);
    ```

    Parquet rejects a qualified path in `ADD COLUMNS`; the change needs `--allow-full-refresh`.

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

| Operation | DuckDB | Spark + Delta | Spark + Parquet | BigQuery |
|-----------|--------|---------------|-----------------|----------|
| Add nullable column | ALTER TABLE | ALTER TABLE | ALTER TABLE | ALTER TABLE |
| Add nullable column with a `default:` | ALTER TABLE | ALTER TABLE, then UPDATE | Full refresh | ALTER TABLE, SET DEFAULT, then UPDATE |
| Add NOT NULL column (with default) | ALTER TABLE | Full refresh | Full refresh | Full refresh |
| Remove column | ALTER TABLE | Table rewrite | Full refresh | ALTER TABLE |
| Widen scalar type | ALTER COLUMN TYPE | Table rewrite | Full refresh | ALTER COLUMN SET DATA TYPE (nullable columns) |
| Relax NOT NULL to nullable | ALTER COLUMN | ALTER COLUMN DROP NOT NULL | Full refresh | ALTER COLUMN DROP NOT NULL |
| Add struct field (nullable) | `ADD COLUMN col.field` | `ADD COLUMNS (col.field)` | Full refresh | Full refresh |
| Remove struct field | `DROP COLUMN col.field` | Full refresh | Full refresh | Full refresh |
| Widen type in struct | ALTER COLUMN TYPE (full struct) | Table rewrite | Full refresh | Full refresh |
| Widen array element type | ALTER COLUMN TYPE | Table rewrite | Full refresh | Full refresh |
| Add field to array-of-structs | ALTER COLUMN TYPE (full type) | `ADD COLUMNS (col.element.field)` | Full refresh | Full refresh |
| Widen map value type | ALTER COLUMN TYPE | Table rewrite | Full refresh | Full refresh (no map type) |
| Change map key type | Full refresh | Full refresh | Full refresh | Full refresh |
| Backfill expression (UPDATE) | UPDATE statement | UPDATE statement | Full refresh | UPDATE statement |

### Recommendations

- **DuckDB** has the most complete schema evolution support. All safe changes can be handled with ALTER TABLE.
- **Spark + Delta** migrates additive changes in place -- new columns, new struct fields, relaxing `NOT NULL` -- and expresses the rest as a table rewrite. Three limits are worth knowing before you plan a migration, and all three are properties of the table smelt creates rather than of Delta itself: a column cannot be added `NOT NULL` or tightened to it; a `DEFAULT` clause cannot ride on the add, so a `default:` becomes an `UPDATE` that fills the rows already there; and dropping or widening a column needs a Delta table feature (`columnMapping`, `enableTypeWidening`) that smelt does not turn on, since enabling one irreversibly raises the table's protocol version. Those changes rewrite the table instead, which needs `--allow-full-refresh`.
- **Spark + Parquet** is the most limited: it takes a new nullable column and nothing else. Consider switching to Delta format if you need frequent schema changes. When a run is refused, the reason names the column and the limitation rather than failing mid-migration.
- **BigQuery** migrates every flat change -- adding and dropping columns, widening a scalar type, relaxing `NOT NULL` -- but nothing that reaches inside a struct or array: GoogleSQL has no dotted `ADD COLUMN`, and its `SET DATA TYPE` refuses a struct that gained or lost a field. Those changes need `--allow-full-refresh`. Two GoogleSQL rules are worth knowing before you plan a migration: a column cannot be *added* `NOT NULL` (nor tightened to it) at all, and an existing `NOT NULL` column cannot be widened -- both resolve to a full refresh, with a message naming the column. When a run is refused, the reason names the exact limitation rather than a generic failure.

### Table format configuration

Spark targets default to Delta format. The target-level `format` field is set in `smelt.yml` under the target's config:

```yaml
# smelt.yml — target-level format
targets:
  spark_prod:
    type: spark
    connect_url: sc://localhost:15002
    schema: main
    format: delta  # or "parquet"
```

To override the format for an individual model, declare `format` in that model's **SQL frontmatter**:

```sql
---
materialization: table
format: parquet
---

SELECT ...
```

See [Project Configuration](../reference/smelt-yml.md) for full target configuration details.

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
