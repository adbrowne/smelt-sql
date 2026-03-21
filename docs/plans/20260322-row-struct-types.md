# Plan: Row/Struct Types for Property Tests

**Date:** 2026-03-22
**Status:** Proposed
**TODO item:** Row/Struct types -- Add ROW(...) and STRUCT(...) constructors to property test generators.

## Context

The property test generators (`crates/smelt-db/tests/prop_helpers/generators.rs`) test smelt's type inference against DuckDB by generating random SQL expressions and comparing inferred types. Currently, composite types (ROW/STRUCT) are not covered despite:

- The **parser** already supporting `ROW(...)` (as `ROW_CONSTRUCTOR` node) and `STRUCT(...)` (as `STRUCT_LITERAL` node) since Phase 8
- The **lexer** already having `ROW_KW` and `STRUCT_KW` tokens
- DuckDB having full support for both `ROW(...)` and `STRUCT(...)` with named fields

However, several pieces are missing:
- The `DataType` enum in `smelt-types` has no `Struct` variant (only `Array`)
- The **AST** has no typed wrappers for `RowConstructor` or `StructLiteral`
- The **type inference** engine does not handle `ROW_CONSTRUCTOR` or `STRUCT_LITERAL` syntax nodes
- The **arrow mapping** falls through to `Unknown` for Arrow `Struct` types
- The **generators** have no ROW/STRUCT expression strategies

## Key Files

| File | Change |
|------|--------|
| `crates/smelt-types/src/lib.rs` | Add `Struct` variant to `DataType` |
| `crates/smelt-parser/src/ast.rs` | Add typed AST wrappers `RowConstructor`, `StructLiteral` |
| `crates/smelt-db/src/type_inference.rs` | Infer types for ROW/STRUCT expressions |
| `crates/smelt-db/tests/prop_helpers/arrow_mapping.rs` | Map `ArrowType::Struct` to `DataType::Struct` |
| `crates/smelt-db/tests/prop_helpers/generators.rs` | Add ROW/STRUCT expression generators |
| `crates/smelt-db/tests/prop_helpers/type_comparison.rs` | Handle struct type comparisons |

## Implementation Steps

### Step 1: Add `Struct` variant to `DataType`

In `crates/smelt-types/src/lib.rs`, add a `Struct` variant alongside the existing `Array`:

```rust
// Complex types
/// Array of elements
Array(Box<DataType>),
/// Struct/Row with named fields
Struct(Vec<(String, DataType)>),
```

A struct is a list of `(field_name, field_type)` pairs. DuckDB's ROW and STRUCT both produce the same underlying struct type. For unnamed fields (e.g., `ROW(1, 2)`), DuckDB auto-generates names like `v1`, `v2`.

Update derived traits: `Struct` contains `Vec` which already implements `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash` (since `String` and `DataType` do), so no manual trait impls needed.

### Step 2: Add AST wrappers for RowConstructor and StructLiteral

In `crates/smelt-parser/src/ast.rs`, add typed AST node wrappers:

- `RowConstructor` wrapping `ROW_CONSTRUCTOR` -- with method `args() -> impl Iterator<Item = Expr>` to iterate field expressions
- `StructLiteral` wrapping `STRUCT_LITERAL` -- with methods:
  - `fields() -> impl Iterator<Item = (Expr, Option<String>)>` to get expression + optional AS name pairs

Add these to the `Expr::cast` dispatch so they are recognized as expressions.

### Step 3: Type inference for ROW/STRUCT

In `crates/smelt-db/src/type_inference.rs`, handle the two new expression types:

**ROW(expr1, expr2, ...):**
- Infer type of each argument expression
- Result type is `DataType::Struct(vec![("v1", type1), ("v2", type2), ...])`
- DuckDB uses 1-based auto-naming: `v1`, `v2`, etc.

**STRUCT(expr1 AS name1, expr2 AS name2, ...):**
- Infer type of each argument expression
- Extract the AS alias for each field
- Result type is `DataType::Struct(vec![("name1", type1), ("name2", type2), ...])`
- For fields without AS, DuckDB uses the expression text as the field name (but for simplicity, we can start with auto-generated names matching ROW behavior, or use the column name if it's a simple column ref)

### Step 4: Arrow mapping for Struct

In `crates/smelt-db/tests/prop_helpers/arrow_mapping.rs`, add a case for `ArrowType::Struct`:

```rust
ArrowType::Struct(fields) => {
    let smelt_fields = fields
        .iter()
        .map(|f| (f.name().clone(), arrow_to_smelt(f.data_type())))
        .collect();
    DataType::Struct(smelt_fields)
}
```

This replaces the current fallback to `Unknown` for struct types.

### Step 5: Property test generators

In `crates/smelt-db/tests/prop_helpers/generators.rs`, add two new expression generators:

**`gen_row_expr`**: Generate `ROW(expr1, expr2, ...)` with 1-4 random sub-expressions from existing base-type columns. Expected type: `Struct` with auto-named fields `v1`, `v2`, ... and corresponding inferred types.

**`gen_struct_expr`**: Generate `STRUCT(expr1 AS name1, expr2 AS name2, ...)` with named fields. Expected type: `Struct` with the given field names and types.

Both generators should:
- Use existing column references and simple literals as field values (avoid deep nesting initially)
- Be added to the top-level expression strategy with appropriate weight (lower than simpler expressions since they're composite)

### Step 6: Type comparison for Struct

In `crates/smelt-db/tests/prop_helpers/type_comparison.rs`, ensure struct comparison works:
- Field names must match exactly
- Field types use the existing compatible-type comparison (e.g., Text/Varchar equivalence)
- Field count must match

## DuckDB Behavior Notes

Key DuckDB behaviors to match:

1. `SELECT ROW(1, 'hello')` returns type `STRUCT(v1 INTEGER, v2 VARCHAR)` -- auto-named fields
2. `SELECT STRUCT_PACK(a := 1, b := 'hello')` is the DuckDB-native syntax; `STRUCT(1 AS a, 'hello' AS b)` also works
3. `SELECT typeof(ROW(1, 2))` returns `STRUCT(v1 INTEGER, v2 INTEGER)`
4. ROW and STRUCT both produce the same Arrow Struct type in results
5. Nested structs work: `ROW(ROW(1, 2), 3)` produces `STRUCT(v1 STRUCT(v1 INTEGER, v2 INTEGER), v2 INTEGER)`

## Potential Complications

- **Field name inference for STRUCT without AS**: DuckDB infers field names from the expression text (e.g., `STRUCT(col_a)` names the field `col_a`). For the initial implementation, we can require explicit AS names in the generator to avoid this complexity.
- **Nested structs**: The generator should initially produce only single-level structs to keep things simple. Nesting can be added later.
- **Display/Debug for Struct DataType**: The `Display` impl for `DataType` will need updating to show struct fields readably.
- **parse_type() in smelt-types**: The `parse_type` function that parses SQL type strings may need to handle `STRUCT(...)` type syntax for CAST expressions, but this is not needed for the generator work (generators produce expressions, not CAST-to-struct).

## Verification

1. `cargo test -p smelt-types` -- DataType::Struct basics
2. `cargo test -p smelt-parser` -- Parser tests for ROW/STRUCT already exist and should still pass
3. `cargo test -p smelt-db` -- Type inference unit tests for ROW/STRUCT
4. `cargo test -p smelt-db --test type_property_tests` -- Property tests with new generators
5. `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests` -- Extended coverage
6. `cargo clippy --all-targets` -- No warnings
7. `cargo fmt --all -- --check` -- Formatting

## Ordering

Steps 1-2 can be done together (types + AST). Step 3 depends on 1-2. Step 4 is independent of 3. Steps 5-6 depend on all prior steps. Suggested implementation order: 1 -> 2 -> 3+4 in parallel -> 5 -> 6.
