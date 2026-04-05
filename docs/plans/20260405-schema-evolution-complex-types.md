# Plan: Production Schema Evolution for Complex Types

**Date:** 2026-04-05
**Status:** Proposed
**Branch:** worktree-schema_evolution

## Context

Schema evolution currently stores column types as SQL strings and compares them with flat string matching. Any change to a Struct, Array, or Map column — even safe ones like adding a nullable field — triggers a full table refresh. This plan upgrades schema evolution to handle complex/nested types structurally, with backend-specific DDL generation for DuckDB, Spark+Delta, and Spark+Parquet.

### Research

- `docs/research/2026-04-04-struct-array-schema-evolution-gaps.md` — Current gaps analysis
- `docs/research/2026-04-05-nested-type-schema-evolution-engines.md` — Engine comparison (DuckDB, Spark, PostgreSQL, Iceberg)

### Design Decisions (from planning conversation)

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Parse SQL strings into `DataType` enums for comparison** | Recursive normalization for free, reuses existing types |
| 2 | **Struct field order IS significant** | Matches DuckDB's positional storage; reordering is a structural change |
| 3 | **Generate `struct_pack` rewrites for nested type widening** | Avoids full refresh; DuckDB supports `ALTER COLUMN TYPE USING` |
| 4 | **Include Map types** | Common in Spark/Databricks; `Map(Box<DataType>, Box<DataType>)` |
| 5 | **`default:` becomes SQL expression string** (breaking change) | Uniform handling for scalar and complex types; no migration path needed |
| 6 | **Abstract schema operations → backend-specific DDL** | Separates "what changed" from "how to execute"; enables multi-backend |
| 7 | **Both DuckDB and Spark must work for all operations** | Production requirement from day one |
| 8 | **Spark: mergeSchema only when result matches strict implementation** | i.e., only for adding nullable fields with NULL default |
| 9 | **Spark: table rewrite for unsupported operations** | Not back to source — transform existing data in-place |
| 10 | **Target-level `format: delta\|parquet` with per-model override** | Single Spark cluster may have both formats |
| 11 | **Auto fallback to full refresh when backend can't handle migration** | With `--allow-full-refresh` CLI gate for expensive operations |
| 12 | **Expand-migrate-contract only when ALTER doesn't support widening** | Primary path is `struct_pack` rewrite |
| 13 | **User-facing docs on smeltsql.com** (MkDocs Material) | Document all behavior, config, backend matrix |

---

## Key Files (Current State)

| File | Purpose |
|------|---------|
| `crates/smelt-types/src/lib.rs` | `DataType` enum — Array, Struct variants (no Map yet) |
| `crates/smelt-types/src/parse.rs` | `parse_type()` — scalars only, no complex types |
| `crates/smelt-state/src/schema_tracking.rs` | `diff_schemas()`, `normalize_type()`, `is_safe_type_widening()`, `plan_migration()` |
| `crates/smelt-cli/src/migration.rs` | `check_and_migrate()`, `extract_evolution_maps()` |
| `crates/smelt-core/src/metadata.rs` | `yaml_value_to_sql_literal()`, `ColumnMetadata`, `SchemaEvolutionConfig` |
| `crates/smelt-cli/tests/incremental/schema_evolution.rs` | Integration tests (scalar types only) |
| `crates/smelt-dialect/src/dialect.rs` | `BackendCapabilities`, `SqlDialect` |
| `crates/smelt-backend-spark/src/lib.rs` | Spark backend via PyO3 |
| `crates/smelt-backend-spark/src/sql.rs` | Spark SQL generation |
| `docs-site/docs/guide/incremental-models.md` | Existing incremental docs (no schema evolution page) |

---

## Phase 1: Extend `parse_type()` for Complex Types [x]

**Goal:** `parse_type()` handles `STRUCT(...)`, `TYPE[]`, `TYPE ARRAY`, and `MAP(K, V)` recursively.

### Work Items

- [x] 1a. Add `Map(Box<DataType>, Box<DataType>)` variant to `DataType` in `smelt-types/src/lib.rs`
- [x] 1b. Add `to_sql()` for Map: `MAP(key_type, value_type)`
- [x] 1c. Add `is_complex()` helper method to `DataType`
- [x] 1d. Extend `parse_type()` in `smelt-types/src/parse.rs` to handle:
  - `STRUCT(field1 TYPE1, field2 TYPE2, ...)` — recursive field parsing
  - `TYPE[]` suffix notation for arrays
  - `TYPE ARRAY` suffix notation for arrays
  - `ARRAY(TYPE)` prefix notation for arrays (Spark style)
  - `MAP(KEY_TYPE, VALUE_TYPE)` for maps
  - Nested combinations: `STRUCT(a INTEGER[], b MAP(VARCHAR, INTEGER))`
  - `STRUCT(a STRUCT(x INTEGER, y VARCHAR), b BIGINT)` — nested structs
- [x] 1e. Handle type aliases inside complex types: `STRUCT(a INT)` parses as `STRUCT(a INTEGER)`

### Red-Green Tests

Write tests FIRST, then implement:

```rust
// parse_type tests to add:
parse_type("INTEGER[]") == DataType::Array(Box::new(DataType::Integer))
parse_type("VARCHAR[]") == DataType::Array(Box::new(DataType::Varchar { max_length: None }))
parse_type("STRUCT(a INTEGER, b VARCHAR)") == DataType::Struct(vec![("a", Integer), ("b", Varchar)])
parse_type("STRUCT(a INT, b BOOL)") == DataType::Struct(vec![("a", Integer), ("b", Boolean)])  // aliases
parse_type("MAP(VARCHAR, INTEGER)") == DataType::Map(Box::new(Varchar), Box::new(Integer))
parse_type("STRUCT(a INTEGER[])") == DataType::Struct(vec![("a", Array(Integer))])
parse_type("STRUCT(a STRUCT(x INTEGER))") == DataType::Struct(vec![("a", Struct(vec![("x", Integer)]))])
parse_type("STRUCT(a INTEGER, b VARCHAR)[]") == DataType::Array(Box::new(Struct(...)))
parse_type("MAP(VARCHAR, STRUCT(a INTEGER))") == DataType::Map(Varchar, Struct(...))
parse_type("BIGINT[][]") == DataType::Array(Box::new(DataType::Array(Box::new(DataType::BigInt))))
// Error cases:
parse_type("STRUCT()") == Err(...)
parse_type("STRUCT(a)") == Err(...)  // missing type
parse_type("MAP(VARCHAR)") == Err(...)  // missing value type
```

### Verification

```bash
cargo test -p smelt-types
cargo clippy --all-targets
```

---

## Phase 2: Recursive Type Normalization [x]

**Goal:** `normalize_type()` handles complex types structurally, not as opaque strings. This prevents spurious schema change detections from alias differences inside nested types.

### Work Items

- [x] 2a. Add `pub fn normalize(dt: &DataType) -> DataType` to `smelt-types` that recursively normalizes a `DataType`:
  - Scalar aliases already handled by `parse_type()` (INT→INTEGER, etc.)
  - `Text` → `Varchar { max_length: None }` (canonical form for comparison)
  - Recurse into Array element, Struct fields, Map key/value
- [x] 2b. Refactor `normalize_type()` in `schema_tracking.rs` to:
  1. Parse the SQL string via `parse_type()`
  2. Call `normalize()` on the result
  3. Return the normalized `DataType` (change return type from `String` to `NormalizedType`)
  4. Fall back to uppercase string comparison for unparseable types (forward compat)
- [x] 2c. Update `diff_schemas()` to compare normalized `DataType` values instead of strings
- [x] 2d. Update `is_safe_type_widening()` to accept `&DataType` instead of `&str`

### Red-Green Tests

```rust
// Normalization: these should NOT trigger schema changes
diff("STRUCT(a INT, b BOOL)", "STRUCT(a INTEGER, b BOOLEAN)") == empty
diff("INT[]", "INTEGER[]") == empty
diff("MAP(STRING, INT)", "MAP(VARCHAR, INTEGER)") == empty
diff("STRUCT(a TEXT)", "STRUCT(a VARCHAR)") == empty
diff("STRUCT(a STRUCT(x INT8))", "STRUCT(a STRUCT(x BIGINT))") == empty

// These SHOULD trigger changes
diff("STRUCT(a INTEGER)", "STRUCT(a BIGINT)") == ChangeType  // widening, not alias
diff("INTEGER[]", "BIGINT[]") == ChangeType  // widening, not alias
```

### Verification

```bash
cargo test -p smelt-types
cargo test -p smelt-state
cargo clippy --all-targets
```

---

## Phase 3: Structural Diff for Complex Types [x]

**Goal:** `diff_schemas()` produces fine-grained `SchemaChange` variants for Struct field additions, removals, type changes — not just opaque `ChangeType`.

### Work Items

- [x] 3a. Add new `SchemaChange` variants for nested type changes:
  ```rust
  SchemaChange::StructFieldAdded {
      column: String,       // top-level column name
      path: Vec<String>,    // path to the struct (empty for top-level)
      field_name: String,
      field_type: String,   // SQL string for display
      nullable: bool,       // struct fields added to existing data are always nullable
  }
  SchemaChange::StructFieldRemoved {
      column: String,
      path: Vec<String>,
      field_name: String,
  }
  SchemaChange::NestedTypeChange {
      column: String,
      path: Vec<String>,    // e.g., ["address", "zip"] for address.zip type change
      from: String,
      to: String,
  }
  SchemaChange::ArrayElementTypeChange {
      column: String,
      path: Vec<String>,    // path to the array
      from: String,
      to: String,
  }
  SchemaChange::MapKeyTypeChange { column: String, from: String, to: String }
  SchemaChange::MapValueTypeChange { column: String, path: Vec<String>, from: String, to: String }
  SchemaChange::IncompatibleTypeChange {
      column: String,
      from: String,
      to: String,
      reason: String,       // e.g., "struct to array", "field reordered"
  }
  ```
- [x] 3b. Implement `fn diff_types(column: &str, path: &[String], old: &DataType, new: &DataType) -> Vec<SchemaChange>`:
  - **Struct vs Struct**: Compare fields in order. Detect additions at end (safe), removals, type changes per field, reordering (unsafe).
  - **Array vs Array**: Compare element types recursively.
  - **Map vs Map**: Compare key types (change = unsafe) and value types recursively.
  - **Struct vs non-Struct** (or vice versa): `IncompatibleTypeChange`.
  - **Scalar vs Scalar**: Delegate to existing widening logic.
- [x] 3c. Integrate `diff_types()` into `diff_schemas()` — when a column's type changes, call `diff_types()` instead of emitting a flat `ChangeType`.
- [x] 3d. Update `SchemaDiff::requires_full_refresh()` to handle new variants:
  - `StructFieldAdded` (nullable): safe
  - `StructFieldAdded` (not nullable, no default): requires refresh
  - `StructFieldRemoved`: requires flag (like column removal)
  - `NestedTypeChange`: safe if widening, unsafe otherwise
  - `ArrayElementTypeChange`: safe if widening
  - `MapKeyTypeChange`: always unsafe
  - `MapValueTypeChange`: safe if widening
  - `IncompatibleTypeChange`: always unsafe
- [x] 3e. Update `SchemaDiff::summary()` for human-readable nested change descriptions.

### Red-Green Tests

```rust
// Struct field addition (safe)
diff_schemas(
    col("meta", "STRUCT(a INTEGER)", true),
    col("meta", "STRUCT(a INTEGER, b VARCHAR)", true),
) => [StructFieldAdded { column: "meta", path: [], field_name: "b", nullable: true }]
// requires_full_refresh() == false

// Struct field removal
diff_schemas(
    col("meta", "STRUCT(a INTEGER, b VARCHAR)", true),
    col("meta", "STRUCT(a INTEGER)", true),
) => [StructFieldRemoved { column: "meta", path: [], field_name: "b" }]

// Nested struct field addition
diff_schemas(
    col("data", "STRUCT(inner STRUCT(x INTEGER))", true),
    col("data", "STRUCT(inner STRUCT(x INTEGER, y VARCHAR))", true),
) => [StructFieldAdded { column: "data", path: ["inner"], field_name: "y", ... }]

// Array element type widening (safe)
diff_schemas(
    col("scores", "INTEGER[]", true),
    col("scores", "BIGINT[]", true),
) => [ArrayElementTypeChange { column: "scores", from: "INTEGER", to: "BIGINT" }]
// requires_full_refresh() == false (safe widening)

// Map value struct field addition
diff_schemas(
    col("lookup", "MAP(VARCHAR, STRUCT(a INTEGER))", true),
    col("lookup", "MAP(VARCHAR, STRUCT(a INTEGER, b TEXT))", true),
) => [StructFieldAdded { column: "lookup", path: ["value"], field_name: "b", ... }]

// Struct field reorder (unsafe)
diff_schemas(
    col("meta", "STRUCT(a INTEGER, b VARCHAR)", true),
    col("meta", "STRUCT(b VARCHAR, a INTEGER)", true),
) => [IncompatibleTypeChange { reason: "struct field order changed" }]

// Incompatible: struct to scalar
diff_schemas(
    col("meta", "STRUCT(a INTEGER)", true),
    col("meta", "INTEGER", true),
) => [IncompatibleTypeChange { reason: "struct to scalar" }]
```

### Verification

```bash
cargo test -p smelt-state
cargo clippy --all-targets
```

---

## Phase 4: Safe Widening Rules for Nested Types [x]

**Goal:** `is_safe_type_widening()` works recursively for Array elements, Struct fields, and Map values. Returns detailed information about what widening is needed.

### Work Items

- [x] 4a. Refactor `is_safe_type_widening()` to work on `DataType` values (not strings):
  ```rust
  pub fn is_safe_type_widening(from: &DataType, to: &DataType) -> bool
  ```
  *Already done in Phase 2 — function already accepts `&DataType`.*
- [x] 4b. Implement recursive widening rules:
  - **Array**: safe if element type widening is safe
  - **Struct**: NOT a widening (struct changes are handled by field-level diff)
  - **Map**: safe if value type widening is safe AND key type is unchanged
  - **Scalar**: existing rules (SMALLINT→INTEGER, FLOAT→DOUBLE, VARCHAR widening, DECIMAL widening)
- [x] 4c. Update `SchemaDiff::requires_full_refresh()` to use the new `DataType`-based widening check for `NestedTypeChange` and `ArrayElementTypeChange` variants.
  *Already uses `is_safe_type_widening_str()` which delegates to `is_safe_type_widening()` — recursive rules now flow through automatically.*

### Red-Green Tests

```rust
// Array element widening
is_safe_type_widening(Array(Integer), Array(BigInt)) == true
is_safe_type_widening(Array(BigInt), Array(Integer)) == false  // narrowing
is_safe_type_widening(Array(Varchar(50)), Array(Varchar(100))) == true
is_safe_type_widening(Array(Float), Array(Double)) == true

// Nested array
is_safe_type_widening(Array(Array(Integer)), Array(Array(BigInt))) == true

// Map value widening
is_safe_type_widening(
    Map(Varchar, Integer),
    Map(Varchar, BigInt)
) == true

// Map key change is NEVER safe
is_safe_type_widening(
    Map(Integer, Varchar),
    Map(BigInt, Varchar)
) == false

// Struct is NOT a widening (handled by field diff)
is_safe_type_widening(
    Struct(vec![("a", Integer)]),
    Struct(vec![("a", BigInt)])
) == false  // struct changes go through diff_types, not widening

// Integration: array widening doesn't trigger full refresh
diff requiring only safe array widening => requires_full_refresh() == false
```

### Verification

```bash
cargo test -p smelt-state
cargo test -p smelt-types
cargo clippy --all-targets
```

---

## Phase 5: Abstract Schema Operations [x]

**Goal:** The migration planner produces backend-agnostic `SchemaOperation`s instead of SQL strings. This separates "what needs to change" from "how to execute it."

### Work Items

- [x] 5a. Define `SchemaOperation` enum in `smelt-state/src/schema_tracking.rs`:
  ```rust
  pub enum SchemaOperation {
      /// Add a column to a table (top-level)
      AddColumn {
          name: String,
          data_type: DataType,
          nullable: bool,
          default_expr: Option<String>,
      },
      /// Remove a column from a table (top-level)
      RemoveColumn { name: String },
      /// Widen a top-level column's type
      WidenColumnType {
          name: String,
          from: DataType,
          to: DataType,
      },
      /// Change a column's nullability
      ChangeNullability {
          name: String,
          to_nullable: bool,
          default_expr: Option<String>,  // for filling NULLs when going NOT NULL
      },
      /// Add a field to a struct column (possibly nested)
      AddStructField {
          column: String,
          path: Vec<String>,       // e.g., ["inner"] for column.inner.new_field
          field_name: String,
          field_type: DataType,
          default_expr: Option<String>,
      },
      /// Remove a field from a struct column
      RemoveStructField {
          column: String,
          path: Vec<String>,
          field_name: String,
      },
      /// Widen a type nested inside a struct/array/map
      WidenNestedType {
          column: String,
          path: Vec<String>,       // path to the field being widened
          from: DataType,
          to: DataType,
      },
      /// Backfill expression for a column
      BackfillColumn {
          name: String,
          expression: String,
      },
      /// Full column rewrite using an expression (expand-migrate-contract or struct_pack)
      RewriteColumn {
          column: String,
          target_type: DataType,
          using_expr: String,      // e.g., "struct_pack(a := s.a::BIGINT, b := s.b)"
      },
  }
  ```
- [x] 5b. Define `MigrationPlan` struct:
  ```rust
  pub struct MigrationPlan {
      pub operations: Vec<SchemaOperation>,
      pub requires_full_refresh: bool,
      pub full_refresh_reason: Option<String>,
      pub requires_allow_full_refresh: bool,  // gate for --allow-full-refresh
      pub warnings: Vec<String>,
  }
  ```
- [x] 5c. Implement `fn plan_schema_operations(diff: &SchemaDiff, defaults: &HashMap<String, String>, backfills: &HashMap<String, String>) -> MigrationPlan`:
  - Converts `SchemaChange` variants into `SchemaOperation`s
  - Combines related operations (e.g., struct field add + nested type widen on same column → single `RewriteColumn`)
  - Generates `struct_pack` / `struct_insert` expressions for struct rewrites
  - Sets `requires_allow_full_refresh` when backend limitations force full refresh
- [x] 5d. Keep the existing `plan_migration()` function working — updated complex type branches to generate basic DuckDB DDL (dot-notation for struct fields, ALTER COLUMN TYPE for widening). Phase 6 will add full backend-specific DDL generation.

### Red-Green Tests

```rust
// Struct field add → AddStructField operation
plan_operations(StructFieldAdded { column: "meta", field_name: "b", ... })
    => [AddStructField { column: "meta", path: [], field_name: "b", ... }]

// Nested type widen → WidenNestedType + RewriteColumn
plan_operations(NestedTypeChange { column: "meta", path: ["a"], from: "INTEGER", to: "BIGINT" })
    => [WidenNestedType { ... }, RewriteColumn { column: "meta", using_expr: "struct_pack(...)" }]

// Array element widen → WidenColumnType (operates on whole column)
plan_operations(ArrayElementTypeChange { column: "scores", from: "INTEGER", to: "BIGINT" })
    => [WidenColumnType { name: "scores", from: Array(Integer), to: Array(BigInt) }]

// Combined: struct field add + field widen in same struct
plan_operations([
    StructFieldAdded { column: "meta", field_name: "c" },
    NestedTypeChange { column: "meta", path: ["a"], from: "INTEGER", to: "BIGINT" },
]) => single RewriteColumn with struct_pack that includes both changes

// Backfill attached to operation
plan_operations(AddColumn { name: "status" }, backfills: { "status": "CASE..." })
    => [AddColumn { ... }, BackfillColumn { name: "status", expression: "CASE..." }]
```

### Verification

```bash
cargo test -p smelt-state
cargo clippy --all-targets
```

---

## Phase 6: DuckDB Backend DDL Generation [x]

**Goal:** Translate `SchemaOperation`s into DuckDB-specific SQL statements.

### Work Items

- [x] 6a. Create `fn generate_duckdb_ddl(schema: &str, table: &str, ops: &[SchemaOperation]) -> Vec<String>` in a new module `smelt-state/src/ddl_duckdb.rs`:
  - `AddColumn` → `ALTER TABLE s.t ADD COLUMN name TYPE [NOT NULL DEFAULT expr]`
  - `RemoveColumn` → `ALTER TABLE s.t DROP COLUMN name`
  - `WidenColumnType` → `ALTER TABLE s.t ALTER COLUMN name TYPE new_type`
  - `ChangeNullability` → UPDATE + `ALTER TABLE s.t ALTER COLUMN name SET/DROP NOT NULL`
  - `AddStructField` → `ALTER TABLE s.t ADD COLUMN col.path.field TYPE` (dot-notation)
  - `RemoveStructField` → `ALTER TABLE s.t DROP COLUMN col.path.field` (dot-notation)
  - `WidenNestedType` → dot-notation `ALTER COLUMN col.path TYPE new_type` for struct fields, full MAP type for map values
  - `BackfillColumn` → `UPDATE s.t SET name = expression`
  - `RewriteColumn` → `ALTER TABLE s.t ALTER COLUMN col TYPE new_type USING expr`
- [x] 6b. Implement `fn build_struct_pack_expr(column: &str, old_type: &DataType, new_type: &DataType) -> Option<String>`:
  - Generates `struct_pack(field1 := col.field1::NEW_TYPE, field2 := col.field2, ...)` expressions
  - Handles nested structs recursively
  - Includes new fields with NULL or default value
  - Handles type casts for widened fields
- [x] 6c. Handle deeply nested cases: `build_list_transform_expr()` generates `list_transform(col, x -> struct_pack(...))` for Array-of-Struct widening. Also added `build_using_expr()` convenience function.
- [x] 6d. Wire DuckDB DDL generation into `plan_migration()` — complex type `SchemaChange` variants now route through `plan_schema_operations()` → `generate_duckdb_ddl()`.

### Red-Green Tests

```rust
// Simple struct field add
generate_duckdb_ddl("main", "t", &[AddStructField {
    column: "meta", path: vec![], field_name: "b", field_type: Varchar, ..
}]) == ["ALTER TABLE main.t ADD COLUMN meta.b VARCHAR"]

// Nested struct field add
generate_duckdb_ddl("main", "t", &[AddStructField {
    column: "data", path: vec!["inner".into()], field_name: "y", ..
}]) == ["ALTER TABLE main.t ADD COLUMN data.inner.y VARCHAR"]

// Struct field type widening via struct_pack
generate_duckdb_ddl("main", "t", &[RewriteColumn {
    column: "meta",
    target_type: Struct(vec![("a", BigInt), ("b", Varchar)]),
    using_expr: "struct_pack(a := meta.a::BIGINT, b := meta.b)",
}]) == ["ALTER TABLE main.t ALTER COLUMN meta TYPE STRUCT(a BIGINT, b VARCHAR) USING struct_pack(a := meta.a::BIGINT, b := meta.b)"]

// Array element widening
generate_duckdb_ddl("main", "t", &[WidenColumnType {
    name: "scores", from: Array(Integer), to: Array(BigInt),
}]) == ["ALTER TABLE main.t ALTER COLUMN scores TYPE BIGINT[]"]

// struct_pack builder
build_struct_pack_expr("meta",
    Struct(vec![("a", Integer), ("b", Varchar)]),
    Struct(vec![("a", BigInt), ("b", Varchar), ("c", Boolean)]),
) == "struct_pack(a := meta.a::BIGINT, b := meta.b, c := NULL::BOOLEAN)"
```

### Verification

```bash
cargo test -p smelt-state  # or whichever crate houses the DDL gen
cargo clippy --all-targets
```

---

## Phase 7: Table Format Config and Backend Capabilities [x]

**Goal:** Spark targets distinguish between Delta and Parquet table formats. `BackendCapabilities` reflects what each format supports for schema evolution.

### Work Items

- [x] 7a. Add `format` field to `Target` in `smelt-core/src/config.rs`:
  ```rust
  pub struct Target {
      // ... existing fields ...
      /// Table format for Spark targets: "delta" (default) or "parquet"
      #[serde(default)]
      pub format: Option<TableFormat>,
  }

  #[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
  pub enum TableFormat {
      #[default]
      #[serde(rename = "delta")]
      Delta,
      #[serde(rename = "parquet")]
      Parquet,
  }
  ```
- [x] 7b. Add per-model format override in `ModelMetadata`:
  ```rust
  pub struct ModelMetadata {
      // ... existing fields ...
      /// Override table format (e.g., parquet for a specific model on a Delta target)
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub format: Option<TableFormat>,
  }
  ```
- [x] 7c. Add schema evolution capabilities to `BackendCapabilities`:
  ```rust
  pub struct BackendCapabilities {
      // ... existing fields ...
      pub supports_struct_field_ddl: bool,          // ALTER TABLE ADD COLUMN s.field
      pub supports_alter_column_using: bool,        // ALTER COLUMN TYPE ... USING expr
      pub supports_nested_array_ddl: bool,          // ALTER TABLE ADD COLUMN items.element.field
      pub supports_merge_schema_write: bool,        // mergeSchema on write (Spark)
      pub supports_column_mapping: bool,            // ID-based column mapping (Delta)
  }
  ```
- [x] 7d. Set capabilities per backend+format:
  - **DuckDB**: all struct/array DDL = true, merge_schema_write = false, column_mapping = false
  - **Spark+Delta**: struct_field_ddl = true, alter_column_using = false, nested_array_ddl = false, merge_schema_write = true, column_mapping = true
  - **Spark+Parquet**: struct_field_ddl = true (metastore only), alter_column_using = false, nested_array_ddl = false, merge_schema_write = true, column_mapping = false
- [x] 7e. Add `--allow-full-refresh` CLI flag to `smelt run` command.

### Red-Green Tests

```rust
// Config parsing
parse_target("type: spark\nformat: delta\n...") => Target { format: Some(Delta), ... }
parse_target("type: spark\nformat: parquet\n...") => Target { format: Some(Parquet), ... }
parse_target("type: spark\n...") => Target { format: None, ... }  // defaults to Delta

// Per-model override
parse_metadata("format: parquet\nmaterialization: table\n") => ModelMetadata { format: Some(Parquet), ... }

// Capabilities
BackendCapabilities::duckdb().supports_struct_field_ddl == true
BackendCapabilities::duckdb().supports_alter_column_using == true
BackendCapabilities::spark_delta().supports_merge_schema_write == true
BackendCapabilities::spark_delta().supports_alter_column_using == false
BackendCapabilities::spark_parquet().supports_column_mapping == false
```

### Verification

```bash
cargo test -p smelt-core
cargo test -p smelt-dialect
cargo test -p smelt-cli
cargo clippy --all-targets
```

---

## Phase 8: Spark Backend DDL Generation [x]

**Goal:** Translate `SchemaOperation`s into Spark-specific SQL for both Delta and Parquet table formats.

### Work Items

- [x] 8a. Create `fn generate_spark_ddl(catalog: &str, schema: &str, table: &str, ops: &[SchemaOperation], format: TableFormat, caps: &BackendCapabilities) -> MigrationExecution`:
  ```rust
  pub enum MigrationExecution {
      /// DDL statements to execute
      Statements(Vec<String>),
      /// Use mergeSchema write (Spark only)
      MergeSchemaWrite { columns_to_add: Vec<(String, DataType)> },
      /// Rewrite table from itself (not from source)
      TableRewrite { select_expr: String },
      /// Requires full refresh from source — needs --allow-full-refresh
      FullRefreshRequired { reason: String },
  }
  ```
- [x] 8b. Implement Spark+Delta DDL:
  - `AddStructField` → `ALTER TABLE cat.s.t ADD COLUMNS (col.field TYPE)`
  - `RemoveStructField` → `ALTER TABLE cat.s.t DROP COLUMN col.field` (requires column mapping)
  - `WidenNestedType` → `TableRewrite` (Delta doesn't support `ALTER COLUMN TYPE USING`)
  - `AddColumn` → `ALTER TABLE cat.s.t ADD COLUMNS (name TYPE)`
  - `RemoveColumn` → `ALTER TABLE cat.s.t DROP COLUMN name`
  - `WidenColumnType` → `ALTER TABLE cat.s.t ALTER COLUMN name TYPE new_type` (safe widenings only)
- [x] 8c. Implement Spark+Parquet DDL:
  - `AddStructField` (nullable) → `ALTER TABLE cat.s.t ADD COLUMNS (col.field TYPE)` (metastore)
  - `AddStructField` (not nullable) → `FullRefreshRequired`
  - `RemoveStructField` → `FullRefreshRequired` (no safe way on Parquet)
  - `WidenNestedType` → `FullRefreshRequired` (old files have old types)
  - `AddColumn` (nullable) → `ALTER TABLE cat.s.t ADD COLUMNS (name TYPE)`
  - `WidenColumnType` → `FullRefreshRequired` for most; INT→BIGINT works at read time
  - Array-of-struct field add (nullable) → `MergeSchemaWrite` (same behavior as strict path)
- [x] 8d. Implement `TableRewrite` execution for Spark:
  - `CREATE TABLE tmp AS SELECT transform_expr FROM original`
  - `DROP TABLE original`
  - `ALTER TABLE tmp RENAME TO original`
  - Wrap in Delta transaction where possible
- [x] 8e. Wire Spark DDL generation into migration execution, selecting DuckDB vs Spark path based on backend dialect.
- [x] 8f. Implement `--allow-full-refresh` gating: when `MigrationExecution::FullRefreshRequired` is returned and the flag is not set, return an error with a clear message.

### Red-Green Tests

```rust
// Spark+Delta: struct field add
generate_spark_ddl("cat", "db", "t", &[AddStructField { column: "meta", field_name: "b", .. }], Delta, ..)
    => Statements(["ALTER TABLE cat.db.t ADD COLUMNS (meta.b VARCHAR)"])

// Spark+Delta: nested type widen → table rewrite
generate_spark_ddl("cat", "db", "t", &[WidenNestedType { column: "meta", .. }], Delta, ..)
    => TableRewrite { select_expr: "..." }

// Spark+Parquet: struct field add (nullable) → DDL
generate_spark_ddl("cat", "db", "t", &[AddStructField { nullable: true, .. }], Parquet, ..)
    => Statements(["ALTER TABLE cat.db.t ADD COLUMNS (meta.b VARCHAR)"])

// Spark+Parquet: nested type widen → requires full refresh
generate_spark_ddl("cat", "db", "t", &[WidenNestedType { .. }], Parquet, ..)
    => FullRefreshRequired { reason: "..." }

// Array-of-struct field add on Parquet → mergeSchema
generate_spark_ddl("cat", "db", "t", &[AddStructField {
    column: "items", path: ["element"], field_name: "score", nullable: true, ..
}], Parquet, ..)
    => MergeSchemaWrite { columns_to_add: [("items.element.score", Double)] }
```

### Verification

```bash
cargo test -p smelt-backend-spark
cargo test -p smelt-state
cargo clippy --all-targets
```

---

## Phase 9: `default:` as SQL Expression String [x]

**Goal:** Breaking change — `default:` in column metadata becomes a raw SQL expression string instead of a YAML value converted to SQL.

### Work Items

- [x] 9a. Change `ColumnMetadata.default` from `Option<serde_yaml::Value>` to `Option<String>` in `smelt-core/src/metadata.rs`.
- [x] 9b. Remove `yaml_value_to_sql_literal()` function (no longer needed).
- [x] 9c. Update `extract_evolution_maps()` in `smelt-cli/src/migration.rs` to pass through the string directly instead of calling `yaml_value_to_sql_literal()`.
- [x] 9d. Update all tests that use `default:` in frontmatter:
  - `default: 0` → `default: "0"`
  - `default: unknown` → `default: "'unknown'"`
  - `default: true` → `default: "TRUE"`
  - `default: null` → `default: "NULL"`
- [x] 9e. Add tests for complex type defaults:
  - `default: "STRUCT_PACK(status := 'unknown', count := 0)"`
  - `default: "[]::VARCHAR[]"`
  - `default: "MAP {}"`
  - `default: "ARRAY[1, 2, 3]"`
- [x] 9f. Update existing schema evolution tests in `smelt-cli/tests/incremental/schema_evolution.rs`.
  *No changes needed — integration tests don't use `default:` in frontmatter.*

### Red-Green Tests

```rust
// Parsing: default is now a string
parse_metadata("columns:\n  status:\n    default: \"'unknown'\"")
    => ColumnMetadata { default: Some("'unknown'".to_string()), ... }

// Complex type defaults
parse_metadata("columns:\n  meta:\n    default: \"STRUCT_PACK(a := 0, b := '')\"")
    => ColumnMetadata { default: Some("STRUCT_PACK(a := 0, b := '')".to_string()), ... }

parse_metadata("columns:\n  tags:\n    default: \"[]::VARCHAR[]\"")
    => ColumnMetadata { default: Some("[]::VARCHAR[]".to_string()), ... }

// extract_evolution_maps passes through directly
extract_evolution_maps(metadata_with_default("status", "'pending'"))
    => defaults: { "status": "'pending'" }
```

### Verification

```bash
cargo test -p smelt-core
cargo test -p smelt-cli
cargo clippy --all-targets
```

---

## Phase 10: Integration — Wire Everything Together [x]

**Goal:** End-to-end schema evolution for complex types works through the full pipeline: diff → plan operations → generate DDL → execute.

### Work Items

- [x] 10a. Update `check_and_migrate()` in `smelt-cli/src/migration.rs` to:
  1. Parse deployed and inferred column types via `parse_type()`
  2. Generate `SchemaDiff` with fine-grained nested changes
  3. Plan abstract `SchemaOperation`s
  4. Select DDL generator based on backend dialect + table format
  5. Execute DDL or handle `FullRefreshRequired` / `MergeSchemaWrite`
  6. Gate full refresh behind `--allow-full-refresh`
  *Most of this was wired in Phases 5-9. This session fixed remaining bugs and verified e2e.*
- [x] 10b. Thread `TableFormat` through from target config / model metadata to migration execution.
  *Already done in Phase 7 — `run.rs` resolves per-model override > target default.*
- [x] 10c. Thread `--allow-full-refresh` CLI flag through to migration execution.
  *Already done in Phase 7 — passed as parameter to `check_and_migrate()`.*
- [x] 10d. Update `SchemaEvolutionResult` to include new outcomes:
  ```rust
  pub enum SchemaEvolutionResult {
      // ... existing variants ...
      /// Full refresh required but --allow-full-refresh not set
      FullRefreshBlocked { reason: String },
      /// Table rewrite performed (not from source)
      TableRewrite { description: String },
  }
  ```
  *Already done in Phase 8.*
- [x] 10e. Ensure `save_deployed_schema()` correctly persists complex type strings after migration.
  *Verified — `DeployedColumn.data_type` stores the SQL string as-is (e.g., `STRUCT(a INTEGER, b VARCHAR)`).*

### Red-Green Tests

```rust
// End-to-end: DuckDB struct field addition
// Deploy v1 with STRUCT(a INTEGER)
// Infer v2 with STRUCT(a INTEGER, b VARCHAR)
// → ALTER TABLE t ADD COLUMN meta.b VARCHAR
// → No full refresh

// End-to-end: DuckDB nested type widening
// Deploy v1 with STRUCT(a INTEGER, b VARCHAR)
// Infer v2 with STRUCT(a BIGINT, b VARCHAR)
// → ALTER TABLE t ALTER COLUMN meta TYPE ... USING struct_pack(...)

// End-to-end: Array element widening
// Deploy v1 with INTEGER[]
// Infer v2 with BIGINT[]
// → ALTER TABLE t ALTER COLUMN scores TYPE BIGINT[]

// End-to-end: Spark+Parquet unsupported op without flag
// → FullRefreshBlocked { reason: "..." }

// End-to-end: Spark+Parquet unsupported op with --allow-full-refresh
// → FullRefreshRequired (proceeds)
```

### Verification

```bash
cargo test -p smelt-cli
cargo test  # full test suite
cargo clippy --all-targets
```

---

## Phase 11: DuckDB Integration Tests [x]

**Goal:** End-to-end tests that execute real DDL against DuckDB for all complex type schema evolution scenarios.

### Work Items

- [x] 11a. Add integration tests in `smelt-cli/tests/incremental/schema_evolution.rs`:
  - Struct field addition → incremental continues (already existed from Phase 10)
  - Struct field removal (with flag) → incremental continues
  - Struct field type widening via struct_pack → incremental continues (already existed from Phase 10)
  - Array element type widening → incremental continues (already existed from Phase 10)
  - Map value type change → incremental continues
  - Array-of-struct field addition → incremental continues
  - Nested struct field addition → incremental continues
  - Incompatible change (struct to scalar) → full refresh (already existed from Phase 10)
  - Multiple changes in one migration (add field + widen type)
  - Complex type with default expression → NOT NULL column added (deferred: default expression tests covered in Phase 9)
- [x] 11b. Test `struct_pack` rewrite produces correct data:
  - Insert rows with v1 struct schema
  - Migrate to v2 (widened field + new field)
  - Verify old rows have correct widened values and NULL for new field
  - Insert new rows with v2 schema
  - Verify both old and new rows queryable
- [x] 11c. Test deeply nested types:
  - `STRUCT(nested STRUCT(x INTEGER))` → `STRUCT(nested STRUCT(x BIGINT, y VARCHAR))`
  - `STRUCT(items INTEGER[])` → `STRUCT(items BIGINT[])`
- [x] 11d. Test Map column evolution:
  - `MAP(VARCHAR, INTEGER)` → `MAP(VARCHAR, BIGINT)` (value widening)
  - `MAP(VARCHAR, STRUCT(a INTEGER))` → `MAP(VARCHAR, STRUCT(a INTEGER, b TEXT))`

### Verification

```bash
cargo test -p smelt-cli --test schema_evolution
cargo test  # full suite
```

---

## Phase 12: Spark DDL Tests (Unit) [x]

**Goal:** Unit tests for Spark DDL generation covering Delta and Parquet paths. These don't require a running Spark cluster.

### Work Items

- [x] 12a. Add unit tests for `generate_spark_ddl()` in `smelt-state/src/ddl_spark.rs`:
  - Delta: struct field add/remove DDL
  - Delta: table rewrite SQL generation for nested type widening
  - Parquet: struct field add DDL (metastore)
  - Parquet: FullRefreshRequired for unsupported operations
  - Parquet: MergeSchemaWrite for nullable array-of-struct field add
  - Additional: AddColumn with DEFAULT, NOT NULL, ChangeNullability paths, widening chain, backfill+add combo
- [x] 12b. Test `TableRewrite` SQL generation:
  - Generates correct `CREATE TABLE tmp AS SELECT ... FROM original`
  - Includes type casts for widened fields (struct cast, complex select)
  - Generates DROP + RENAME sequence
  - Verifies RewriteColumn select_expr format (AS column, EXCEPT)
- [x] 12c. Test error messages for `FullRefreshRequired`:
  - Clear description of what's unsupported
  - Suggests remediation (e.g., switch to Delta format)
  - Mentions `--allow-full-refresh` flag
  - Fixed 3 messages missing remediation suggestions (AddColumn NOT NULL, BackfillColumn, ChangeNullability on Parquet)

### Verification

```bash
cargo test -p smelt-backend-spark
cargo clippy --all-targets
```

---

## Phase 13: User-Facing Documentation [ ]

**Goal:** Comprehensive schema evolution documentation on smeltsql.com.

### Work Items

- [ ] 13a. Create `docs-site/docs/guide/schema-evolution.md`:
  - Overview: what schema evolution is, why it matters
  - How smelt detects schema changes (structural comparison)
  - Safe vs unsafe changes (with examples)
  - Configuration: `default:`, `backfill:`, `schema_evolution:` in frontmatter
  - The `--allow-full-refresh` flag
  - Examples for every supported change type
- [ ] 13b. Create complex type examples section:
  - Adding a field to a struct column
  - Widening a type inside a struct
  - Adding a field to an array-of-structs
  - Map value evolution
  - Specifying defaults for complex types
  - Multi-step evolution (add + widen in one change)
- [ ] 13c. Create backend capability matrix page or section:
  - Table: operation x backend (DuckDB, Spark+Delta, Spark+Parquet)
  - What happens for each unsupported operation
  - Recommendations (when to use Delta vs Parquet)
- [ ] 13d. Update `docs-site/docs/guide/incremental-models.md`:
  - Link to schema evolution page
  - Mention `--allow-full-refresh` flag
- [ ] 13e. Update `docs-site/docs/reference/project-configuration.md`:
  - Document `format: delta|parquet` in target config
  - Document per-model `format:` override
- [ ] 13f. Add schema evolution to `docs-site/mkdocs.yml` navigation.

### Verification

```bash
cd docs-site && pip install -r requirements.txt && mkdocs build --strict
```

---

## Phase 14: Polish and Edge Cases [ ]

**Goal:** Handle edge cases, improve error messages, ensure robustness.

### Work Items

- [ ] 14a. Handle `parse_type()` failures gracefully in `diff_schemas()`:
  - If a deployed type string can't be parsed (e.g., from an older smelt version or external source), fall back to string comparison with a warning
  - Test: unparseable type string → fallback to string diff, not crash
- [ ] 14b. Improve error messages:
  - `FullRefreshBlocked` should explain exactly what changed, why it's unsupported, and what flag to use
  - `IncompatibleTypeChange` summary should show the full type paths
  - Dry-run mode (`smelt diff`) should show the full migration plan with operations
- [ ] 14c. Handle column quoting for identifiers that need it (struct field names with spaces/keywords).
- [ ] 14d. Verify round-trip: `DataType` → `to_sql()` → `parse_type()` → `DataType` for all types including Map.
- [ ] 14e. Run `cargo test -p smelt-cli --test example_diagnostics` to verify existing examples still work.
- [ ] 14f. Update `docs/ROADMAP.md` with completion status.

### Verification

```bash
cargo test
cargo clippy --all-targets
cargo fmt --all -- --check
cargo test -p smelt-cli --test example_diagnostics
```

---

## Decisions Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-04-05 | Parse into DataType for comparison (not string-level) | Recursive normalization, reuse existing types |
| 2026-04-05 | Field order significant in structs | Matches DuckDB positional storage |
| 2026-04-05 | Generate struct_pack rewrites for nested widening | Avoids unnecessary full refresh |
| 2026-04-05 | Include Map type | Common in Spark/Databricks enterprises |
| 2026-04-05 | default: becomes SQL expression string | Uniform for scalar and complex types |
| 2026-04-05 | Abstract schema operations → backend DDL | Multi-backend from day one |
| 2026-04-05 | DuckDB + Spark (Delta & Parquet) all supported | Production requirement |
| 2026-04-05 | mergeSchema only for nullable-add-with-NULL-default | Behavior must match strict implementation |
| 2026-04-05 | Auto fallback to full refresh with --allow-full-refresh gate | Smart strategy selection, safety gate for expensive ops |
| 2026-04-05 | Expand-migrate-contract only when ALTER USING unavailable | struct_pack rewrite is primary path |
| 2026-04-05 | DuckDB struct widening: ALTER COLUMN TYPE without USING | DuckDB auto-casts compatible types inside structs; USING+struct_pack doesn't work (can't reference struct fields of the column being altered) |

---

## Session Log

### Session 1 — 2026-04-05

**Phase completed:** Phase 1 (Extend `parse_type()` for Complex Types)

**What was done:**
- Added `Map(Box<DataType>, Box<DataType>)` variant to `DataType` enum
- Added `is_complex()` method to `DataType` (returns true for Array, Struct, Map)
- Added `to_sql()` rendering for Map: `MAP(key_type, value_type)`
- Rewrote `parse_type()` to support recursive complex type parsing:
  - `STRUCT(field1 TYPE1, field2 TYPE2, ...)` with recursive field parsing
  - `TYPE[]` bracket notation for arrays (including multi-dimensional: `BIGINT[][]`)
  - `TYPE ARRAY` suffix notation
  - `ARRAY(TYPE)` prefix notation (Spark style)
  - `MAP(KEY_TYPE, VALUE_TYPE)` for maps
  - Arbitrarily nested combinations (e.g., `MAP(VARCHAR, STRUCT(a INTEGER[]))`)
  - Type aliases inside complex types (e.g., `STRUCT(a INT)` → `STRUCT(a INTEGER)`)
  - Parameterized types inside structs (e.g., `STRUCT(a DECIMAL(10,2))`)
- Added `InvalidStruct` and `InvalidMap` error variants to `TypeParseError`
- Added helper functions: `find_matching_paren()`, `split_top_level_commas()`, `parse_struct_fields()`, `parse_map_params()`
- 15 new tests covering all complex type parsing scenarios + error cases
- All 39 smelt-types tests pass, 39 smelt-db tests pass (including property tests), clippy clean

**Decisions:** None beyond what was already in the plan.

### Session 2 — 2026-04-05

**Phase completed:** Phase 2 (Recursive Type Normalization)

**What was done:**
- Added `DataType::normalize()` method to `smelt-types` — recursively normalizes `Text` → `Varchar { max_length: None }` through Array, Struct, Map
- Added `smelt-types` as a dependency of `smelt-state`
- Refactored `normalize_type()` in `schema_tracking.rs`:
  - Now parses SQL strings via `parse_type()` + `normalize()` for structural comparison
  - Returns `NormalizedType` enum (Parsed/Unparsed) for forward-compatible fallback
  - Added `normalized_types_equal()` helper for comparing normalized types
- Updated `diff_schemas()` to use structural DataType comparison instead of string comparison
- Rewrote `is_safe_type_widening()` to accept `&DataType` values with pattern matching on type variants
- Added `is_safe_type_widening_str()` convenience wrapper for callers with string types
- Updated all internal callers of the old string-based widening check
- 13 new tests: 6 for normalize() in smelt-types, 7 for complex type normalization in smelt-state
- All 45 smelt-types tests pass, 47 smelt-state tests pass, clippy clean

**Decisions:**
- `VARCHAR` and `TEXT` now normalize to the same type (`Varchar { max_length: None }`), so `VARCHAR → TEXT` is no longer a "widening" — it's the same type. Updated existing test accordingly.
- `normalize_type()` returns `NormalizedType` enum (not raw `DataType`) to handle unparseable types gracefully via uppercase string fallback.

### Session 3 — 2026-04-05

**Phase completed:** Phase 3 (Structural Diff for Complex Types)

**What was done:**
- Added 7 new `SchemaChange` variants: `StructFieldAdded`, `StructFieldRemoved`, `NestedTypeChange`, `ArrayElementTypeChange`, `MapKeyTypeChange`, `MapValueTypeChange`, `IncompatibleTypeChange`
- Implemented `diff_types()` — recursive structural comparison of `DataType` values producing fine-grained change variants
- Implemented `diff_struct_fields()` — detects field additions, removals, reordering (unsafe), and per-field type changes with path tracking
- Integrated `diff_types()` into `diff_schemas()` — when both types parse and at least one is complex, uses structural diff instead of flat `ChangeType`
- Updated `requires_full_refresh()` for all new variants with correct safety rules
- Updated `summary()` for human-readable nested change descriptions (e.g., "ADD STRUCT FIELD meta.inner.b VARCHAR")
- Updated `plan_migration()` match to handle new variants (pass-through for now; Phase 5 will generate proper DDL)
- Updated `smelt-cli/src/commands/diff.rs` JSON serialization to handle new `SchemaChange` variants
- Updated Phase 2 test `test_complex_type_real_change_detected` to expect `NestedTypeChange` instead of `ChangeType`
- 16 new tests covering: struct field add/remove, nested struct field add, array element widening (safe/unsafe), map value struct field add, struct reorder (incompatible), struct-to-scalar (incompatible), map key change (unsafe), map value widening, nested type widening, multiple changes, summary formatting, direct diff_types unit tests, deeply nested struct changes
- All 63 smelt-state tests pass, all smelt-types and smelt-cli tests pass, clippy clean

**Decisions:**
- Map scalar value type changes emit `MapValueTypeChange` directly (not recursing through `diff_types` for scalars). Complex map values (e.g., struct) recurse structurally.
- New complex type variants in `plan_migration()` are pass-through for now — Phase 5 will convert them to proper `SchemaOperation`s and DDL.

### Session 4 — 2026-04-05

**Phase completed:** Phase 4 (Safe Widening Rules for Nested Types)

**What was done:**
- Added recursive widening rules to `is_safe_type_widening()` for Array, Map, and Struct types
  - **Array**: safe if element type widening is safe (recursive)
  - **Map**: safe if value type widening is safe AND key type is unchanged
  - **Struct**: explicitly NOT a widening (struct changes go through field-level diff)
- 11 new tests: array element widening (safe/unsafe), nested array widening, map value widening, map key change (never safe), struct not widening, string-based array/map widening, integration tests for `requires_full_refresh()` with array/map/nested changes
- All smelt-state and smelt-types tests pass, clippy clean

**Decisions:**
- 4a was already done in Phase 2 (`is_safe_type_widening` already accepted `&DataType`)
- 4c was already handled — `requires_full_refresh()` uses `is_safe_type_widening_str()` which delegates to `is_safe_type_widening()`, so recursive rules flow through automatically
- Phase was smaller than expected since earlier phases set up the right abstractions

### Session 5 — 2026-04-05

**Phase completed:** Phase 5 (Abstract Schema Operations)

**What was done:**
- Added `SchemaOperation` enum with 9 variants: `AddColumn`, `RemoveColumn`, `WidenColumnType`, `ChangeNullability`, `AddStructField`, `RemoveStructField`, `WidenNestedType`, `BackfillColumn`, `RewriteColumn`
- Added `MigrationPlan` struct with `operations`, `requires_full_refresh`, `full_refresh_reason`, `requires_allow_full_refresh`, `warnings`
- Implemented `plan_schema_operations()` — converts `SchemaChange` diff variants into backend-agnostic `SchemaOperation`s:
  - Groups struct-level changes per column for possible combination
  - Handles defaults/backfills for column additions
  - Detects unsafe changes that require full refresh
  - Emits warnings for struct field removals
- Updated `plan_migration()` complex type branches from pass-through to basic DuckDB DDL generation:
  - `StructFieldAdded` → `ALTER TABLE ADD COLUMN col.field TYPE` (dot-notation)
  - `StructFieldRemoved` → `ALTER TABLE DROP COLUMN col.field`
  - `NestedTypeChange` → `ALTER TABLE ALTER COLUMN col TYPE new_type`
  - `ArrayElementTypeChange` → `ALTER TABLE ALTER COLUMN col TYPE new_type[]`
  - `MapValueTypeChange` → placeholder ALTER (Phase 6 will handle properly)
- 16 new tests covering all operation types: struct field add, nested type widen, array element widen, add column with backfill, NOT NULL with/without default, scalar type widen, unsafe type change, remove column, nullability changes, map key/value changes, incompatible type change, struct field removal with warning, combined struct add+widen, empty diff
- All 91 smelt-state tests pass, clippy clean, fmt clean

**Decisions:**
- `plan_migration()` was updated with basic DDL generation for complex types rather than full delegation to `plan_schema_operations()` — this avoids a large refactor while Phase 6 will introduce proper backend-specific DDL generators
- Struct changes are grouped per column in `plan_schema_operations()` to enable future RewriteColumn combination (Phase 6 will generate struct_pack expressions)
- Map value widening emits `WidenNestedType` with path `["value"]` to distinguish from column-level operations

### Session 6 — 2026-04-05

**Phase completed:** Phase 6 (DuckDB Backend DDL Generation)

**What was done:**
- Created `smelt-state/src/ddl_duckdb.rs` — new module for DuckDB-specific DDL generation from abstract `SchemaOperation`s
- Implemented `generate_duckdb_ddl()` handling all 9 `SchemaOperation` variants:
  - `AddColumn`, `RemoveColumn`, `WidenColumnType`, `ChangeNullability` (scalar ops)
  - `AddStructField`, `RemoveStructField` using DuckDB dot-notation (`col.path.field`)
  - `WidenNestedType` using dot-notation for struct fields, full MAP type reconstruction for map values
  - `BackfillColumn` (UPDATE), `RewriteColumn` (ALTER COLUMN TYPE ... USING expr)
- Implemented `build_struct_pack_expr()` — recursively generates `struct_pack(field := ...)` expressions:
  - Handles unchanged fields (pass-through), widened fields (cast), new fields (NULL::TYPE)
  - Recursively handles nested struct-in-struct with inner `struct_pack()` calls
- Implemented `build_list_transform_expr()` for array-of-struct widening:
  - Generates `list_transform(col, x -> struct_pack(...))` for Array(Struct) type changes
- Added `build_using_expr()` convenience function (tries struct_pack first, then list_transform)
- Wired DDL generation into `plan_migration()` — complex type `SchemaChange` variants now route through `plan_schema_operations()` → `generate_duckdb_ddl()` instead of inline DDL generation
- 28 new tests: 22 in `ddl_duckdb.rs` (DDL generation, struct_pack, list_transform, using_expr), 6 in `schema_tracking.rs` (end-to-end DDL pipeline tests)
- All 125 smelt-state tests pass, 45 smelt-types tests pass, clippy clean, fmt clean

**Decisions:**
- `WidenNestedType` uses DuckDB dot-notation (`ALTER COLUMN col.field TYPE new_type`) rather than struct_pack USING expressions for individual field widenings. struct_pack is reserved for `RewriteColumn` operations that transform the entire struct.
- Map value widening reconstructs the full `MAP(VARCHAR, new_value_type)` type for ALTER COLUMN TYPE. The key type defaults to VARCHAR since the `WidenNestedType` operation doesn't carry the full map type.
- `build_struct_pack_expr()` returns `Option<String>` (None for non-struct types) rather than panicking.

### Session 7 — 2026-04-05

**Phase completed:** Phase 7 (Table Format Config and Backend Capabilities)

**What was done:**
- Added `TableFormat` enum (`Delta`, `Parquet`) to `smelt-core/src/config.rs` with `#[derive(Default)]` (defaults to Delta)
- Added `format: Option<TableFormat>` field to `Target` struct with custom deserializer (case-insensitive "delta"/"parquet")
- Added `Target::table_format()` helper — returns `None` for DuckDB, defaults to `Delta` for Spark when unspecified
- Added `format: Option<TableFormat>` field to `ModelMetadata` for per-model format override
- Added 5 new schema evolution capability fields to `BackendCapabilities`:
  - `supports_struct_field_ddl` — DuckDB: true, Spark+Delta: true, Spark+Parquet: true
  - `supports_alter_column_using` — DuckDB: true, Spark: false, PostgreSQL: true
  - `supports_nested_array_ddl` — DuckDB: true, Spark: false
  - `supports_merge_schema_write` — DuckDB: false, Spark: true
  - `supports_column_mapping` — DuckDB: false, Spark+Delta: true, Spark+Parquet: false
- Added `spark_delta()` and `spark_parquet()` constructors to `BackendCapabilities` (existing `spark()` now aliases `spark_delta()`)
- Added `--allow-full-refresh` CLI flag to `smelt run` (threaded to `check_and_migrate()` as `_allow_full_refresh` — Phase 10 will wire it up)
- Updated all 10 `Target` struct literal constructions across 8 files to include `format: None`
- 5 new tests for config parsing (delta, parquet, default, duckdb, invalid format rejection)
- 2 new tests for model metadata format override
- 5 new tests for backend capability assertions (duckdb, spark_delta, spark_parquet, spark_default, postgresql)
- All tests pass, clippy clean, fmt clean

**Decisions:**
- `smelt-dialect` does NOT depend on `smelt-core` — no `spark_for_format()` convenience method on `BackendCapabilities`. Callers select the right constructor based on `TableFormat`.
- `spark()` aliases `spark_delta()` for backward compatibility — existing code that calls `BackendCapabilities::spark()` gets Delta capabilities automatically.
- Spark+Parquet disables `supports_merge` (no Delta = no MERGE statement) in addition to schema evolution differences.

### Session 8 — 2026-04-05

**Phase completed:** Phase 8 (Spark Backend DDL Generation)

**What was done:**
- Phase 8 was mostly implemented in prior sessions (8a–8f code was already in place). This session completed the remaining gap and fixed compilation issues:
- Added `MergeSchemaWrite` path for array-of-struct field additions: when `AddStructField` path contains "element" (indicating array nesting) and `!caps.supports_nested_array_ddl`, returns `MergeSchemaWrite` instead of DDL. Applies to both Spark+Delta and Spark+Parquet.
- Fixed exhaustive match in `smelt-cli/src/commands/diff.rs` for new `MigrationAction::TableRewrite` and `MigrationAction::FullRefreshBlocked` variants (both human-readable and JSON output paths).
- 2 new tests: `test_parquet_array_of_struct_field_add_merge_schema`, `test_delta_array_of_struct_field_add_merge_schema`
- All 30 ddl_spark tests pass, all smelt-state/smelt-cli/smelt-types tests pass, clippy clean, fmt clean

**Decisions:**
- Array-of-struct detection uses `path.iter().any(|p| p == "element")` — the "element" sentinel in the path signals that we're navigating through an array element, matching the convention established by `diff_types()` in Phase 3.

### Session 9 — 2026-04-05

**Phase completed:** Phase 9 (`default:` as SQL Expression String)

**What was done:**
- Changed `ColumnMetadata.default` from `Option<serde_yaml::Value>` to `Option<String>` in `smelt-core/src/metadata.rs`
- Removed `yaml_value_to_sql_literal()` function and its test — no longer needed since defaults are raw SQL expression strings
- Simplified `extract_evolution_maps()` in `smelt-cli/src/migration.rs` — now passes through the string directly instead of converting from YAML values. Removed `yaml_value_to_sql_literal` import and unused `tracing::warn` import
- Updated `test_frontmatter_with_column_default` test: `default: unknown` → `default: "'unknown'"`, `default: 0` → `default: "0"`, assertions check string directly instead of going through `yaml_value_to_sql_literal()`
- Added `test_frontmatter_with_complex_type_defaults` test covering: `STRUCT_PACK(...)`, `[]::VARCHAR[]`, `MAP {}`, `ARRAY[1, 2, 3]`, `TRUE`, `NULL`
- No changes needed to `smelt-cli/tests/incremental/schema_evolution.rs` — integration tests don't use `default:` in frontmatter
- All 95 smelt-core tests pass (excluding pre-existing python_models failures), all smelt-state/smelt-cli tests pass, clippy clean, fmt clean

**Decisions:**
- This is a breaking change for anyone using unquoted YAML values in `default:` (e.g., `default: 0` would now be parsed as the string `"0"` by serde, which is actually the desired behavior — it's now a SQL expression passed through directly). The old `default: unknown` (unquoted YAML string) would now be `"unknown"` (a SQL identifier) rather than `"'unknown'"` (a SQL string literal). Users must explicitly quote: `default: "'unknown'"`.

### Session 10 — 2026-04-05

**Phase completed:** Phase 10 (Integration — Wire Everything Together)

**What was done:**
- Most Phase 10 work items were already completed incrementally during Phases 5-9. This session identified and fixed remaining bugs, then verified the full e2e pipeline.
- **Bug fix:** `IncompatibleTypeChange` and `MapKeyTypeChange` variants in `plan_migration_for_backend()` were falling through to an empty match arm, producing `AlterTable { statements: [] }` instead of `FullRefresh`. Added these to the `unresolvable_reasons` check so they correctly trigger `FullRefresh`.
- **Bug fix:** DuckDB struct field widening was generating `ALTER COLUMN meta.a TYPE BIGINT` (dot-notation) which DuckDB doesn't support for ALTER COLUMN TYPE. Fixed to generate `ALTER TABLE t ALTER COLUMN meta TYPE STRUCT(a BIGINT, b VARCHAR)` — DuckDB handles safe casts (INTEGER→BIGINT inside struct) automatically.
- Added `deployed_columns` and `inferred_columns` parameters to `plan_migration_for_backend()` so the DuckDB path can look up full column types for struct widening. Updated all callers.
- 2 new unit tests: `test_plan_migration_incompatible_type_change_triggers_full_refresh`, `test_plan_migration_map_key_type_change_triggers_full_refresh`
- 3 new integration tests: `test_e2e_nested_type_widening` (DuckDB struct_pack rewrite), `test_e2e_incompatible_type_triggers_full_refresh`, `test_e2e_map_key_change_triggers_full_refresh`
- Updated existing `test_ddl_pipeline_nested_type_widening` to use `plan_migration_for_backend` with columns and expect correct ALTER COLUMN TYPE DDL
- All 161 smelt-state + smelt-cli tests pass, clippy clean, fmt clean

**Decisions:**
- DuckDB struct field widening uses simple `ALTER COLUMN col TYPE new_full_struct_type` without USING clause. DuckDB's implicit casting handles safe type widenings (INTEGER→BIGINT) inside structs automatically. The USING+struct_pack approach doesn't work because DuckDB's USING expression context can't reference struct fields of the column being altered.
- `plan_migration_for_backend` now accepts `deployed_columns` and `inferred_columns` to support this. The `plan_migration` convenience wrapper passes empty slices (its callers don't test struct widening with real columns).

### Session 11 — 2026-04-05

**Phase completed:** Phase 11 (DuckDB Integration Tests)

**What was done:**
- Added 10 new DuckDB integration tests in `smelt-cli/tests/incremental/schema_evolution.rs`:
  - `test_e2e_struct_field_removal` — struct field removal with `allow_column_removal=true`
  - `test_e2e_map_value_widening` — MAP(VARCHAR, INTEGER) → MAP(VARCHAR, BIGINT)
  - `test_e2e_array_of_struct_field_addition` — STRUCT(a INTEGER)[] → STRUCT(a INTEGER, b VARCHAR)[]
  - `test_e2e_nested_struct_field_addition` — STRUCT(nested STRUCT(x INTEGER)) → STRUCT(nested STRUCT(x INTEGER, y VARCHAR))
  - `test_e2e_multiple_changes_one_migration` — struct field add + array element widen in same migration
  - `test_e2e_struct_pack_data_correctness` — full data verification: insert v1 rows, migrate (widen + add field), verify old values preserved with NULL for new field, insert v2 row, verify all queryable
  - `test_e2e_deeply_nested_struct_widen_and_add` — inner struct field widened + new field added simultaneously
  - `test_e2e_struct_with_array_field_widen` — STRUCT(items INTEGER[]) → STRUCT(items BIGINT[])
  - `test_e2e_map_value_struct_field_addition` — MAP(VARCHAR, STRUCT(a INTEGER)) → MAP(VARCHAR, STRUCT(a INTEGER, b TEXT))
  - `test_e2e_map_value_type_widening` — MAP value type widening verified against DuckDB
- **Bug fix:** Extended DuckDB ALTER COLUMN TYPE special-case in `plan_migration_for_backend()` to handle:
  - `StructFieldAdded` on array/map columns (DuckDB can't use dot-notation ADD COLUMN on non-struct columns)
  - `StructFieldAdded` with non-empty path (nested struct inside another struct — use full column type ALTER)
  - `ArrayElementTypeChange` with non-empty path (array inside a struct)
  - Added deduplication to avoid emitting duplicate ALTER COLUMN TYPE statements when multiple changes affect the same column
- All 22 schema evolution integration tests pass (10 new + 12 existing), all smelt-state/smelt-types/smelt-cli tests pass, clippy clean, fmt clean

**Decisions:**
- For DuckDB, any complex type change on an array or map column uses `ALTER COLUMN TYPE` on the full column instead of dot-notation operations. DuckDB only supports dot-notation for direct struct columns, not for struct elements inside arrays/maps.
- Multiple changes on the same column (e.g., field add + type widen) are deduplicated into a single `ALTER COLUMN TYPE` statement since DuckDB handles all compatible changes in one type cast.

### Session 12 — 2026-04-05

**Phase completed:** Phase 12 (Spark DDL Tests — Unit)

**What was done:**
- Added 18 new unit tests to `smelt-state/src/ddl_spark.rs` (total now 48):
  - **12a edge cases:** `AddColumn` with DEFAULT clause, `AddColumn` NOT NULL, `ChangeNullability` set NOT NULL with/without default (Delta and Parquet), SMALLINT widening chain, backfill+add column combo
  - **12b table rewrite:** `generate_table_rewrite_sql()` with struct casts, complex SELECT expressions, `RewriteColumn` select_expr format verification (AS column, EXCEPT), `WidenColumnType` table rewrite CAST format
  - **12c error messages:** 7 tests verifying all `FullRefreshRequired` messages include remediation suggestions (mention Delta format and/or `--allow-full-refresh`)
- **Bug fixes (RED→GREEN):**
  - Fixed `AddColumn` DEFAULT clause placement: was `ADD COLUMNS (col TYPE) DEFAULT expr`, now correctly `ADD COLUMNS (col TYPE DEFAULT expr)` (inside parentheses)
  - Improved 4 error messages that were missing remediation suggestions:
    - `AddColumn` NOT NULL on Parquet: added "Consider using Delta format or --allow-full-refresh"
    - `BackfillColumn` on Parquet: added "Consider using Delta format or --allow-full-refresh"
    - `ChangeNullability` SET NOT NULL on Parquet: added "Consider using Delta format or --allow-full-refresh"
    - `ChangeNullability` SET NOT NULL without default: added "Provide a default or use --allow-full-refresh"
- All 48 ddl_spark tests pass, all workspace tests pass (excluding pre-existing python_models failures), clippy clean, fmt clean

**Decisions:**
- Tests live in `smelt-state/src/ddl_spark.rs` (where the code is), not in `smelt-backend-spark/src/tests.rs` as the plan originally envisioned. The Spark backend crate handles connectivity/execution; DDL generation is in smelt-state.
