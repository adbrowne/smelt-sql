---
feature: schema_evolution
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Schema Evolution

> **What this is.** A normative spec for smelt's schema evolution system — change classification (safe vs. unsafe), `ALTER TABLE` strategy, `smelt diff` behavior, stored schema format, and backend capability matrix. Out of scope: where state is stored (see `run_state.md`); environment reuse (see `virtual_environments.md`); output-level change detection (see `output_fingerprint.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### `smelt diff` output

```
smelt diff [--select <selector>] [--json]
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

`--json` produces a machine-readable object. All enum values use snake_case:

```json
{
  "models": {
    "<model_name>": {
      "status": "new" | "unchanged" | "changed" | "removed",
      "changes": [
        {
          "type": "add_column" | "remove_column" | "change_type" | "change_nullability"
               | "struct_field_added" | "struct_field_removed" | "nested_type_change"
               | "array_element_type_change" | "map_key_type_change" | "map_value_type_change"
               | "incompatible_type_change",
          "column": "<col_name>",
          "data_type": "<type string>",
          "nullable": true | false,
          "from": "<previous type or value>",
          "to": "<new type or value>",
          "path": "<nested field path, if applicable>",
          "reason": "<explanation string>"
        }
      ],
      "warnings": ["<string>"],
      "risk": {
        "requires_full_refresh": true | false,
        "has_column_removals": true | false,
        "migration_action": "no_change" | "alter_table" | "full_refresh" | "full_refresh_blocked"
                          | "requires_column_removal_flag" | "table_rewrite",
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

The `columns:` frontmatter map (full shape: see `models.md` §"`columns:` — column metadata") includes two per-column keys consumed by the schema-evolution path:

| Key | Description |
|-----|-------------|
| `default` | SQL literal used as the DEFAULT expression when adding a NOT NULL column. Supplying `default:` **or** `backfill:` (or both) reclassifies a NOT NULL column add as Safe; both populate existing rows. |
| `backfill` | SQL expression applied in an UPDATE statement after the column is added, to populate existing rows. Like `default:`, its presence reclassifies a NOT NULL column add as Safe. |

```yaml
columns:
  status:
    default: "'active'"        # SQL expression for new NOT NULL column default
    backfill: "lower(status)"  # SQL UPDATE expression run after column addition
```

## Semantics

### Stored schemas

When a model is materialized by `smelt run` or `smelt build`, smelt writes the deployed schema to `.smelt/schemas/<model_name>.json`. The stored schema contains:

- `version`: Integer, incremented by 1 on each successful migration
- `deployment_timestamp`: ISO 8601 timestamp
- `model_hash`: SHA-256 of the model SQL at deploy time
- `columns`: Array of `{name, data_type, nullable}` objects

If `.smelt/schemas/` does not exist, `smelt diff` reports all models as `new`.

### Stale schema cleanup

After a successful `smelt run` or `smelt build`, smelt scans `.smelt/schemas/` and deletes any `.json` entry whose model name is not in the set of models discovered in the current project. A model is discovered if a corresponding `.sql` file exists anywhere in the project's scanned tree (discovery is project-wide; `paths:` only strips address prefixes — see `architecture.md` §"Resolution"). Stale entries arise when a model file is deleted without a rebuild.

**Why this matters:** without cleanup, `smelt diff` will permanently report deleted models as `REMOVED` even after the user has removed the file and rebuilt. The cleanup runs only after a *successful* build — a failed build does not trigger cleanup so as not to destroy the deployed-schema record for a model whose SQL has a syntax error.

This cleanup applies only to `.json` schema files, not to the live database. Smelt does not drop the corresponding database table automatically when a model is deleted; that is left to the user.

`smelt diff` does not require a live database connection. It reads the stored schemas and runs type inference on the current model SQL offline.

### Change classification

The following change types are detected:

| Change | Classification |
|--------|---------------|
| Add nullable column | **Safe** — ALTER TABLE ADD COLUMN |
| Add NOT NULL column with `default:` and/or `backfill:` | **Safe** — ALTER TABLE ADD with DEFAULT (and an UPDATE backfill when `backfill:` is set) |
| Add NOT NULL column with neither `default:` nor `backfill:` | **Blocked** — requires `--allow-full-refresh` |
| Remove column | **Blocked by default** — requires `--allow-column-removal` |
| Widen scalar type (see widening table) | **Safe** — ALTER TABLE ALTER COLUMN TYPE |
| Narrow scalar type | **Blocked** — requires `--allow-full-refresh` |
| Incompatible type change (struct↔scalar) | **Blocked** — requires `--allow-full-refresh` |
| Change NOT NULL → NULL | **Safe** — ALTER TABLE ALTER COLUMN |
| Change NULL → NOT NULL | **Blocked** — requires `--allow-full-refresh` unless `default:` and/or `backfill:` is set |
| Add nullable struct field | **Safe** |
| Remove struct field | **Blocked** — requires `--allow-column-removal` |
| Reorder struct fields | **Blocked** — requires `--allow-full-refresh` |
| Widen array element type | **Safe** if element widening is safe |
| Change map key type | **Always blocked** — requires `--allow-full-refresh` |
| Widen map value type | **Safe** if value widening is safe |

### NOT NULL column-add reclassification

Adding a NOT NULL column (or tightening NULL → NOT NULL) is Safe **iff at least one of `default:` or `backfill:` is declared** on that column. Both populate existing rows so the NOT NULL constraint can hold: `default:` supplies the value via an `ADD COLUMN ... DEFAULT`, `backfill:` populates existing rows with an UPDATE after the add. When **both** are present, the column is added with the `default:` expression and the `backfill:` UPDATE then overwrites existing rows (the backfill takes precedence for pre-existing rows; the default governs subsequent inserts). When **neither** is present, the change stays **Blocked** and requires `--allow-full-refresh` — smelt will not silently insert NULL into a NOT NULL column.

### Safe scalar type widenings

A type change is a safe widening if it cannot cause data loss:

| From | To |
|------|----|
| `SMALLINT` | `INTEGER`, `BIGINT` |
| `INTEGER` | `BIGINT` |
| `FLOAT` | `DOUBLE` |
| `CHAR` | `VARCHAR` |
| `VARCHAR(n)` | `VARCHAR` (unbounded) |
| `DECIMAL(p,s)` | `DECIMAL(p2,s2)` where s2 ≥ s **and** (p2 − s2) ≥ (p − s) — the scale must not shrink and the integer-digit capacity must not shrink (`p2 ≥ p` follows). `DECIMAL(5,0) → DECIMAL(6,4)` is **not** safe: integer digits drop 5 → 2. |
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

**Offline diff.** `smelt diff` does not connect to the live database. Stored schemas in `.smelt/schemas/` serve as the deployed-state snapshot. This enables CI checks without database credentials and means `smelt diff` reflects the *last deployed* schema, not the current live one. If the database was modified outside of smelt, the stored schema will diverge from reality. Live-connection diff was rejected because it requires credentials in CI environments and creates non-determinism when the database is being modified concurrently.

**ALTER TABLE over full refresh as default.** The system prefers non-destructive ALTER TABLE operations for additive changes. Full table recreation is reserved for changes that cannot be expressed as additive DDL. This avoids unnecessary data movement on large tables. Full-refresh-by-default was rejected because additive changes (adding a nullable column, widening a type) have no correctness requirement for rewriting existing rows — requiring a full refresh would penalise the common case.

**Column removal is always opt-in.** Dropping columns is a destructive operation that cannot be reversed. Requiring `--allow-column-removal` makes the intent explicit in pipelines and prevents accidental column drops from model renames or typos.

**`backfill:` for NOT NULL column additions.** Adding a NOT NULL column requires populating existing rows. Rather than silently inserting NULL (which would violate the constraint) or always requiring a full refresh, smelt supports `default:` (column default for new rows) and `backfill:` (UPDATE expression for existing rows) as first-class evolution primitives. Silent NULL insertion was rejected because it violates the declared constraint; always-full-refresh was rejected because it forces large table rewrites for what is often a small migration.

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
- **`model_hash` not used for change detection.** The stored model SQL hash is recorded but not used by `smelt diff` to decide whether a model needs re-running — only schema column differences trigger migration actions. The principled successor is the *semantic* output-fingerprint (`output_fingerprint.md`): a raw SQL hash over-reports change (any formatting edit re-runs), whereas the fingerprint re-runs only on a genuine output change. Wiring fingerprint-based change detection into reuse is owned by `virtual_environments.md`.
- **Schema migration and fingerprint reuse are complementary, not alternatives.** A fingerprint match proves *row* equivalence for the same inputs; it does not by itself guarantee the deployed *physical* schema matches. Under `state.mode: environments`, reuse of a table additionally requires that no physical migration is needed, or that one is applied via the `MigrationAction` path here (`virtual_environments.md` §"Reuse decision" condition 4). The two systems compose: this spec classifies the physical change, the fingerprint classifies the logical-output change.
- **`backfill:` and `default:` undocumented in user guide.** The column evolution annotations are implemented but absent from the schema evolution user guide page.
- **Struct field reordering detection.** Whether changing struct field order is detected as `IncompatibleTypeChange` or silently ignored depends on the comparison implementation. Current behavior undocumented in user guide.
- **Exit code for blocked migrations.** When a run is blocked by `RequiresColumnRemovalFlag` or `FullRefreshBlocked`, the exit code is non-zero but the specific code (1 vs. other) is not specified.
- **`.smelt/schemas/` format settling.** The on-disk JSON format of stored schemas and update timing on partial runs are still implementation-defined; the `.smelt/` layout and manifest (run IDs, parallelism, recovery) are owned by `run_state.md`. Stale-schema cleanup semantics are specified above.
- **Reflection-sourced HOF outputs follow source-schema evolution.** A column added to (or removed from) a source schema propagates to the `List<ColumnRef>` produced by `smelt.columns_of(t)` on the next compilation; HOF chains derived from that list (`coalesce_numeric`, schema-driven SELECT lists, and similar derivations defined in `meta_language.md`) reflect the updated column set automatically. The propagation is a compile-time refresh, not a runtime migration: existing deployed schemas of models that consume the reflected output go through the normal `smelt diff` / migration-action path. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Generator-emitted models participate in schema evolution.** Models emitted by a generator file (per `meta_language.md` §"Multi-model production") have stored schemas in `.smelt/schemas/<emitted_model_name>.json` exactly as hand-authored models do, keyed by the emitted `smelt.<path>` identifier. Column additions, removals, and type changes on a generator-emitted model trigger the same `MigrationAction` path as hand-authored models. A `ModelDef.body` whose synthesised column list changes between runs (e.g. because the loader YAML added a row, or `smelt.columns_of` on a referenced source now returns more columns) produces an `AddColumn` / `RemoveColumn` diff on the affected emission; the diff is anchored at the generator file's emitting `ModelDef` literal in editor diagnostics. A generator emission that disappears between runs (the underlying YAML row was removed, the generator's body now returns fewer `ModelDef`s) is treated as a removed model — the stored schema is staled per the existing cleanup rule. Tracked in `docs/plans/20260509-meta-language-overall.md`.

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
  - `output_fingerprint.md` — semantic output-fingerprint (the principled change-detection successor to `model_hash`)
  - `virtual_environments.md` — fingerprint-keyed reuse, which composes with this spec's physical migration
  - `run_state.md` — the `.smelt/` layout that owns `schemas/`
