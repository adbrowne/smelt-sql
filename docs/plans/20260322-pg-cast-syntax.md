# Plan: PostgreSQL `::` Cast Syntax in Property Test Generators

**Date**: 2026-03-22
**Status**: Proposed

## Context

The property test generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently only generate `CAST(expr AS type)` syntax for cast expressions. PostgreSQL (and DuckDB) also support the `expr::type` shorthand, which is a common pattern in real SQL.

The smelt parser already fully supports `::` cast syntax:
- **Lexer**: `DOUBLE_COLON` token defined in `syntax_kind.rs` (line 115)
- **Parser**: Handles `expr::type` as a postfix operator in `parser.rs` (line 1264), wrapping it in `CAST_EXPR` node
- **AST**: `CastExpr` has `is_double_colon_cast()` method to distinguish the two forms (ast.rs line 1367)
- **Type inference**: `infer_cast_type()` in `type_inference.rs` works on `CastExpr` nodes regardless of syntax form

The gap is purely in the test generator -- we should generate both forms to verify that the parser and type inference handle `::` casts correctly end-to-end.

## Key Files

| File | Change |
|------|--------|
| `crates/smelt-db/tests/prop_helpers/generators.rs` | Add `::` cast generation alongside `CAST()` |

## Implementation Steps

### 1. Add a boolean flag for cast syntax style

In the `ExprKind::Cast` match arm (line 813), use the existing `func_idx` (or a separate random parameter) to choose between `CAST(expr AS type)` and `expr::type` syntax.

The simplest approach: use `func_idx` bit to select syntax. Since `func_idx` is already used to pick the target type, we can use a higher bit (e.g., `func_idx / cast_options.len() % 2`) to pick the syntax form.

### 2. Modify the Cast arm in `gen_typed_expr`

Current code (line 844-845):
```rust
Some(TypedExpr {
    sql: format!("CAST({} AS {cast_type})", col.name),
    alias,
    expected_smelt_type: smelt_type.clone(),
})
```

New code:
```rust
let use_pg_cast = (func_idx / cast_options.len()) % 2 == 1;
let sql = if use_pg_cast {
    format!("({})::{cast_type}", col.name)
} else {
    format!("CAST({} AS {cast_type})", col.name)
};
Some(TypedExpr {
    sql,
    alias,
    expected_smelt_type: smelt_type.clone(),
})
```

Note: Parentheses around the expression in `(expr)::type` ensure correct precedence when the expression is a column reference or more complex expression.

### 3. Also update `BaseType::cast_sql` for CTE source columns

The `cast_sql()` method (line 80) generates the CTE column definitions. These could also use `::` syntax, but this is lower priority since they are not the expressions under test. Consider adding a separate `cast_sql_pg()` method or leaving as-is for simplicity in the first pass.

**Recommendation**: Leave `cast_sql()` unchanged in the first pass. The CTE column definitions are not what we're testing -- they just set up typed columns. The `ExprKind::Cast` arm is what exercises the parser's cast handling.

### 4. Type name compatibility

DuckDB accepts the same type names in both `CAST(x AS TYPE)` and `x::TYPE` syntax, so the existing `cast_options` type names (`DOUBLE`, `INTEGER`, `BIGINT`, `VARCHAR`, `TIMESTAMP`, `DATE`) will work with `::` syntax without changes.

## Testing and Verification

1. **Run property tests** to confirm both syntax forms produce correct types:
   ```bash
   cargo test -p smelt-db --test type_property_tests
   ```

2. **Manual spot check** -- run with high case count to get good coverage of the `::` path:
   ```bash
   PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
   ```

3. **Verify parser round-trip** -- the generated `expr::type` SQL should parse to a `CAST_EXPR` node with `is_double_colon_cast() == true`, and type inference should return the same type as the `CAST()` form.

4. **Run full CI checks**:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets
   cargo test
   ```

## Risks and Edge Cases

- **Precedence**: `col::INTEGER + 1` may parse differently than `CAST(col AS INTEGER) + 1`. Using parentheses `(col)::INTEGER` avoids ambiguity. However, DuckDB handles bare `col::INTEGER` fine for simple column refs, so parentheses may be optional. Start with parentheses for safety.
- **DECIMAL type**: `col::DECIMAL(10,2)` -- verify DuckDB accepts parameterized types with `::`. If not, exclude `DECIMAL` from `::` cast targets.
- **No parser changes needed**: The parser already supports `::` syntax, so this is purely a generator change.

## Scope

This is a small, self-contained change to a single file. Estimated effort: under 30 minutes of implementation.
