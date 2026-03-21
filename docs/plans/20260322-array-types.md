# Array Types for Property Tests

## Context

The property test generators in `crates/smelt-db/tests/prop_helpers/generators.rs` have no coverage for array types. The TODO at `docs/TODO.md` lists this as a gap under "Types":

> **Array types** -- Add ARRAY literals, ARRAY_AGG, array subscript, array slice

The parser already supports ARRAY literals (`ARRAY[1, 2, 3]`), array subscripts (`expr[index]`), and array slices (`expr[start:end]`) via `ARRAY_LITERAL`, `ARRAY_SUBSCRIPT`, and `ARRAY_SLICE` syntax nodes. The type system has `DataType::Array(Box<DataType>)`. Type inference handles `ARRAY_AGG` (returns `Array(arg_type)`) but does **not** yet handle `ARRAY_LITERAL`, `ARRAY_SUBSCRIPT`, or `ARRAY_SLICE` nodes -- those are missing from `type_inference.rs`. The Arrow mapping already handles `List -> Array`.

## Key Files

- `crates/smelt-db/tests/prop_helpers/generators.rs` -- Add array expression generators
- `crates/smelt-db/src/type_inference.rs` -- Add inference for ARRAY_LITERAL, ARRAY_SUBSCRIPT, ARRAY_SLICE
- `crates/smelt-db/tests/prop_helpers/arrow_mapping.rs` -- Already handles `List -> Array`, no changes needed
- `crates/smelt-db/tests/prop_helpers/divergences.rs` -- May need entries if DuckDB returns unexpected types
- `crates/smelt-parser/src/ast.rs` -- Already has `ArraySubscript`, `ArraySlice` AST nodes (read-only)
- `crates/smelt-parser/src/parser.rs` -- Already parses ARRAY syntax (read-only)
- `crates/smelt-types/src/lib.rs` -- Already has `DataType::Array(Box<DataType>)` (read-only)
- `crates/smelt-types/src/functions.rs` -- Already has `SqlFunction::ArrayAgg` (read-only)

## Implementation Steps

### Step 1: Type inference for ARRAY_LITERAL

Add a handler in `infer_expression_type()` for `ARRAY_LITERAL` CST nodes:

- Walk child expressions of the ARRAY literal
- Infer the type of the first non-Unknown element
- Return `DataType::Array(Box::new(element_type))`
- For empty `ARRAY[]`, return `DataType::Array(Box::new(DataType::Unknown))`

DuckDB behavior: `SELECT typeof(ARRAY[1, 2, 3])` returns `INTEGER[]`. The element type follows normal promotion rules.

### Step 2: Type inference for ARRAY_SUBSCRIPT

Add a handler for `ARRAY_SUBSCRIPT` nodes:

- Infer the type of the array expression (the left side)
- If it's `DataType::Array(inner)`, return the inner type
- Otherwise return `DataType::Unknown`

DuckDB behavior: `SELECT typeof(ARRAY[1, 2, 3][1])` returns `INTEGER`.

### Step 3: Type inference for ARRAY_SLICE

Add a handler for `ARRAY_SLICE` nodes:

- Infer the type of the array expression
- If it's `DataType::Array(inner)`, return `DataType::Array(inner)` (slice preserves array type)
- Otherwise return `DataType::Unknown`

DuckDB behavior: `SELECT typeof(ARRAY[1, 2, 3][1:2])` returns `INTEGER[]`.

### Step 4: Add ARRAY_AGG to property test generators

Add to `core_functions()` in `generators.rs`:

```rust
FuncDesc {
    name: "ARRAY_AGG",
    input: FuncInput::AnyAggregate,
    extra_args: &[],
    prepend_literal: None,
    output_type: DataType::Unknown, // arg-dependent: Array(arg_type)
},
```

Update `function_return_type()`:

```rust
"ARRAY_AGG" => DataType::Array(Box::new(arg_type.clone())),
```

Since ARRAY_AGG is an aggregate, `assemble_cte_query()` will automatically wrap it in an aggregate-only query. The Arrow mapping already converts `List(field)` to `Array(inner)`.

### Step 5: Add ARRAY literal expression kind

Add a new `ExprKind::ArrayLiteral` variant to the generator:

```rust
ExprKind::ArrayLiteral,
```

In `generate_expr()`, handle it by emitting `ARRAY[<cast_value>, <cast_value>]` using a column's base type. For example, for an Integer column:

```sql
ARRAY[CAST(42 AS INTEGER), CAST(42 AS INTEGER)]
```

Expected smelt type: `DataType::Array(Box::new(col.data_type.clone()))`.

Add it to `expr_kind_strategy()` with weight 1.

### Step 6: Add array subscript expression kind

Add `ExprKind::ArraySubscript`:

In `generate_expr()`, build on ArrayLiteral:

```sql
ARRAY[CAST(42 AS INTEGER), CAST(42 AS INTEGER)][1]
```

Expected smelt type: the element type (e.g., `DataType::Integer`).

Note: This requires Step 1-2 (type inference) to be complete, since smelt needs to infer through the ARRAY literal and then the subscript.

Add to `expr_kind_strategy()` with weight 1.

### Step 7: Add array slice expression kind

Add `ExprKind::ArraySlice`:

```sql
ARRAY[CAST(42 AS INTEGER), CAST(42 AS INTEGER)][1:2]
```

Expected smelt type: `DataType::Array(Box::new(DataType::Integer))`.

Requires Step 3 (type inference for slices).

Add to `expr_kind_strategy()` with weight 1.

## Potential Divergences

- **ARRAY_AGG return type**: DuckDB returns `List(element_type)` via Arrow, which maps to `Array(element_type)`. Should match smelt's inference. No divergence expected.
- **Empty ARRAY[]**: DuckDB infers `INTEGER[]` for untyped empty arrays. Smelt would infer `Array(Unknown)`. Avoid generating empty arrays in the property tests to sidestep this.
- **ARRAY literal element promotion**: If elements have mixed types (e.g., `ARRAY[1, 2.5]`), DuckDB promotes to the widest type. The generator avoids this by using homogeneous literals.

## Testing and Verification

1. After each step, run `cargo test -p smelt-db --test type_property_tests` to check for regressions.
2. Run `cargo clippy --all-targets` to ensure no warnings.
3. For deeper coverage: `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference`.
4. Manual smoke test: verify DuckDB agrees on types for representative queries:
   ```sql
   SELECT typeof(ARRAY[1, 2, 3]);           -- INTEGER[]
   SELECT typeof(ARRAY[1, 2, 3][1]);        -- INTEGER
   SELECT typeof(ARRAY[1, 2, 3][1:2]);      -- INTEGER[]
   SELECT typeof(ARRAY_AGG(x)) FROM (SELECT 1 AS x);  -- INTEGER[]
   ```

## Commit Strategy

- **Commit 1**: Type inference for ARRAY_LITERAL, ARRAY_SUBSCRIPT, ARRAY_SLICE (Steps 1-3)
- **Commit 2**: ARRAY_AGG in generators (Step 4)
- **Commit 3**: ArrayLiteral, ArraySubscript, ArraySlice expression kinds (Steps 5-7)
- Update `docs/TODO.md` to mark "Array types" as complete
