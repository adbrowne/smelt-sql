# Plan: JSON function canonicalization, type inference, and property tests

## Context

JSON functions differ across PostgreSQL, DuckDB, and Spark. smelt's philosophy is logical/physical separation — users write logical SQL, smelt rewrites for the target engine. This plan canonicalizes JSON functions internally and adds correct type inference, with property tests against DuckDB.

## Canonical JSON Function Design

### Safe canonicalization (same semantics across backends)

| Canonical smelt name | PostgreSQL | DuckDB | Spark | Return type |
|---|---|---|---|---|
| `JSON_OBJECT` | `json_build_object('k',v,...)` | `json_object('k',v,...)` | N/A (use `to_json(named_struct(...))`) | Text |
| `JSON_ARRAY` | `json_build_array(1,2,3)` | `json_array(1,2,3)` | N/A (use `to_json(array(...))`) | Text |
| `TO_JSON` | `to_json(val)` | `to_json(val)` | `to_json(col)` *(struct/array only)* | Text |
| `JSON_ARRAY_LENGTH` | `json_array_length(j)` | `json_array_length(j)` | `json_array_length(j)` | BigInt |
| `JSON_OBJECT_KEYS` | `json_object_keys(j)` | `json_keys(j)` | `json_object_keys(j)` | Array(Text) |

### Extraction (JSONPath internally)

| Canonical smelt name | PostgreSQL | DuckDB | Spark | Return type |
|---|---|---|---|---|
| `JSON_EXTRACT` | `col -> 'key'` / `col #> path` | `json_extract(col, path)` / `col -> 'key'` | N/A | Text |
| `JSON_EXTRACT_TEXT` | `col ->> 'key'` / `col #>> path` | `json_extract_string(col, path)` / `col ->> 'key'` | `get_json_object(col, path)` | Text |

The `->` and `->>` operators will be recognized and mapped to `JSON_EXTRACT` / `JSON_EXTRACT_TEXT`.
The `#>` and `#>>` operators do the same with array-path syntax.

### Boolean operators

| Canonical smelt name | PostgreSQL | DuckDB | Spark | Return type |
|---|---|---|---|---|
| `JSON_CONTAINS` | `@>` | `json_contains(a, b)` | *compile error* | Boolean |

`@>` and `<@` operators map to `JSON_CONTAINS` (with argument order swap for `<@`).

### Spark incompatibilities (flagged at compile time, future work)

- `TO_JSON(scalar)` — Spark only supports struct/array/map
- `JSON_CONTAINS` / `@>` / `<@` — no Spark equivalent
- `JSON_OBJECT` / `JSON_ARRAY` — need rewrite to `to_json(named_struct(...))` / `to_json(array(...))`

## Implementation Steps

### 0. Commit plan to `docs/plans/`

Save this plan as `docs/plans/20260321-json-canonicalization.md` before starting implementation.

### 1. Add new SqlFunction variants (`crates/smelt-types/src/functions.rs`)

Add to the enum, `ALL_FUNCTIONS`, `name()`, `from_name()`, and `category()`:
- `JsonObject` — recognize both `JSON_OBJECT` and `JSON_BUILD_OBJECT`
- `JsonArray` — recognize both `JSON_ARRAY` and `JSON_BUILD_ARRAY`
- `JsonExtract` — recognize `JSON_EXTRACT` and `JSON_EXTRACT_PATH`
- `JsonExtractText` — recognize `JSON_EXTRACT_TEXT`, `JSON_EXTRACT_STRING`, `JSON_EXTRACT_PATH_TEXT`, `GET_JSON_OBJECT`
- `JsonArrayLength` — recognize `JSON_ARRAY_LENGTH`
- `JsonObjectKeys` — recognize `JSON_OBJECT_KEYS` and `JSON_KEYS`
- `JsonContains` — recognize `JSON_CONTAINS`

Remove the old `JsonBuildObject`, `JsonBuildArray`, `ToJsonb`, `RowToJson` variants (replaced by the canonical names). Keep `ToJson`.

### 2. Add JSON operator type inference (`crates/smelt-db/src/type_inference.rs`)

In `infer_binary_expr_type`, add before `_ => None`:
```
"->", "#>" => Text (JSON_EXTRACT)
"->>", "#>>" => Text (JSON_EXTRACT_TEXT)
"@>" => Boolean (JSON_CONTAINS)
"<@" => Boolean (JSON_CONTAINS, reversed)
```

### 3. Add JSON function type inference (`crates/smelt-db/src/type_inference.rs`)

In `infer_function_type`, update the JSON match arm:
- `JsonObject`, `JsonArray`, `ToJson`, `JsonExtract`, `JsonExtractText` → Text, nullable
- `JsonArrayLength` → BigInt, nullable
- `JsonObjectKeys` → Array(Text), nullable
- `JsonContains` → Boolean, nullable

### 4. Add JSON to property test generators (`crates/smelt-db/tests/prop_helpers/generators.rs`)

Add JSON functions to `core_functions()` (DuckDB-compatible names for testing):
- `TO_JSON(col)` — AnyScalar input → Text
- `JSON_ARRAY_LENGTH(json_literal)` — use a JSON string literal → BigInt
- `JSON_OBJECT('key', col)` — use DuckDB's `json_object` syntax → Text
- `JSON_ARRAY(col)` — use DuckDB's `json_array` syntax → Text

Add a `JsonOp` variant to `ExprKind` for testing `->` and `->>` operators:
- Generate `CAST('{"a":1,"b":"hello","c":true}' AS JSON) -> 'a'` → Text
- Generate `CAST('{"a":1,"b":"hello","c":true}' AS JSON) ->> 'a'` → Text

### 5. Keep json_type_check test in duckdb_oracle.rs

The test we already added confirms DuckDB JSON → Varchar via Arrow.

### 6. Update TODO.md

Mark JSON functions and JSON operators as done.

## Files to Modify

- `crates/smelt-types/src/functions.rs` — New/renamed SqlFunction variants, from_name aliases
- `crates/smelt-db/src/type_inference.rs` — JSON operator + function type inference
- `crates/smelt-db/tests/prop_helpers/generators.rs` — JSON function/operator generators
- `crates/smelt-db/tests/prop_helpers/duckdb_oracle.rs` — Keep json_type_check test
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — Any new divergences
- `docs/TODO.md` — Update checklist

## Verification

```bash
cargo clippy --all-targets
cargo test
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
```
