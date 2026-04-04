# Research: Struct/Array Support in Schema Evolution

**Date**: 2026-04-04
**Topic**: Whether the parser-type-testing-completeness plan (Phase 12 Struct, Phase 11 Array) creates follow-on work for schema evolution
**Branch**: parser-type-testing-completeness
**Commit**: e48eed3

## Summary

Schema evolution stores column types as SQL strings (e.g., `"STRUCT(a INTEGER, b VARCHAR)"`, `"INTEGER[]"`), not `DataType` enums. Complex types (Struct, Array) flow through `DataType::to_sql()` into string form and are compared as opaque strings with no structural analysis. This means any change to a Struct or Array column -- even safe ones like adding a field or widening an element type -- triggers a full table refresh. There is also no way to specify defaults or backfill expressions for complex types via frontmatter.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-types/src/lib.rs` | `DataType` enum with `Array`/`Struct` variants | L64-66 |
| `crates/smelt-types/src/lib.rs` | `to_sql()` serializes Array/Struct to SQL strings | L149-156 |
| `crates/smelt-state/src/schema_tracking.rs` | `DeployedColumn` stores `data_type: String` | L15-23 |
| `crates/smelt-state/src/schema_tracking.rs` | `normalize_type()` -- no Array/Struct handling | L180-193 |
| `crates/smelt-state/src/schema_tracking.rs` | `is_safe_type_widening()` -- no Array/Struct rules | L196-214 |
| `crates/smelt-state/src/schema_tracking.rs` | `plan_migration()` -- migration generation | L290-453 |
| `crates/smelt-cli/src/migration.rs` | `columns_from_inferred()` -- bridge from DataType to String | L69-81 |
| `crates/smelt-core/src/metadata.rs` | `yaml_value_to_sql_literal()` -- rejects sequences/mappings | L93-116 |
| `crates/smelt-cli/tests/incremental/schema_evolution.rs` | Schema evolution tests (scalar types only) | L9-204 |

## Architecture & Data Flow

Type inference produces `DataType` enums (including `Array(Box<DataType>)` and `Struct(Vec<(String, DataType)>)`). These are converted to SQL strings via `DataType::to_sql()` at the schema persistence boundary:

```
type_inference → DataType::Array(Integer)
                     ↓ to_sql()
              "INTEGER[]"
                     ↓
         DeployedColumn { data_type: "INTEGER[]" }
                     ↓ serde_json
         .smelt/schemas/{model}.json
```

When schema evolution runs, it compares the stored string against the newly inferred string via `normalize_type()` and `is_safe_type_widening()`. Both functions operate on flat string matching with no awareness of nested type structure.

## Current Behavior

### What works

- **Round-trip**: `DataType::Struct(...)` → `to_sql()` → `"STRUCT(a INTEGER, b TEXT)"` is deterministic and well-formed (smelt-types/src/lib.rs:150-156)
- **Change detection**: If a Struct/Array column changes at all, it is correctly detected as a `SchemaChange::ChangeType` because the strings differ
- **Full refresh fallback**: Since `is_safe_type_widening()` returns false for any unrecognized type pair, Struct/Array changes always produce a `FullRefresh` migration action

### Gaps

1. **No structural comparison for Struct types**: `"STRUCT(a INTEGER, b VARCHAR)"` vs `"STRUCT(a INTEGER, b VARCHAR, c BOOLEAN)"` (field addition) is treated the same as `"STRUCT(a INTEGER)"` vs `"STRUCT(x DOUBLE)"` (incompatible change). Both trigger full refresh.

2. **No structural comparison for Array types**: `"INTEGER[]"` vs `"BIGINT[]"` could be a safe widening (element type widened), but is treated as unsafe because `is_safe_type_widening()` only handles scalar types.

3. **No normalization for complex types**: `normalize_type()` maps aliases like `INT` → `INTEGER`, but `"STRUCT(a INT)"` is NOT normalized to `"STRUCT(a INTEGER)"`. Alias differences inside nested types could cause spurious schema change detections.

4. **No defaults for complex types**: `yaml_value_to_sql_literal()` (metadata.rs:108-113) explicitly rejects YAML sequences and mappings, so there is no way to specify a `default:` or `backfill:` for Array or Struct columns in model frontmatter. Adding a NOT NULL Array/Struct column always requires full refresh.

5. **No test coverage**: Schema evolution tests (schema_evolution.rs) only use scalar types (INTEGER, DOUBLE, VARCHAR). No tests exercise Array or Struct column changes.

6. **Field ordering sensitivity**: `"STRUCT(a INTEGER, b VARCHAR)"` vs `"STRUCT(b VARCHAR, a INTEGER)"` would be detected as a type change even though the logical content is the same. (In practice this is unlikely since `to_sql()` serializes in definition order, but could arise if schemas come from different sources.)

## Related Patterns

The pattern of converting `DataType` to SQL strings at the persistence boundary is deliberate -- it avoids needing `Serialize`/`Deserialize` on `DataType` and keeps schema storage backend-agnostic. The `is_safe_type_widening()` function follows a whitelist approach (only explicitly listed widenings are safe), which is conservative but correct.

The `is_decimal_widening()` function (schema_tracking.rs:239-268) shows the pattern for structured type comparison: it parses `DECIMAL(P,S)` from strings and compares precision/scale. A similar approach could parse Array element types or Struct field lists from their SQL string representations.

## Test Coverage

- **Schema evolution tests** (`crates/smelt-cli/tests/incremental/schema_evolution.rs`): 4 tests, all using scalar types (INTEGER, DOUBLE, VARCHAR). No Array/Struct coverage.
- **Schema tracking unit tests** (`crates/smelt-state/src/schema_tracking.rs:560+`): Test safe/unsafe widening for scalar types and VARCHAR/DECIMAL. No complex type tests.
- **Struct DuckDB validation** (`crates/smelt-db/tests/struct_duckdb_validation.rs`): Validates type inference against DuckDB but does not test schema evolution.

## Open Questions

1. **How common are Struct/Array columns in real dbt/smelt pipelines?** If they are rare, the current full-refresh-on-any-change behavior may be acceptable for now.

2. **Should `normalize_type()` recurse into nested types?** This would prevent spurious changes from alias differences inside Struct fields or Array elements, but adds complexity.

3. **Is field-order sensitivity in Struct comparison a real concern?** Since `to_sql()` always serializes in definition order, this only matters if deployed schemas come from external sources (e.g., introspecting an existing database).

4. **Should the YAML default/backfill limitation be addressed?** One option: allow SQL expression strings as defaults for complex types (e.g., `default: "ARRAY[]"` or `default: "ROW(0, '')"`) without trying to parse YAML sequences/mappings.
