# Plan: Scalar Subquery Property Test Generators

**Date**: 2026-03-22
**Status**: Proposed
**TODO item**: "Scalar subqueries -- Generate `(SELECT ...)` in expression position (none covered currently)"

## Context

The property test generators in `crates/smelt-db/tests/prop_helpers/generators.rs` cover many expression kinds (column refs, casts, functions, binary ops, CASE, BETWEEN, IN, window functions, JSON operators) but do not yet generate scalar subqueries. Scalar subqueries like `(SELECT MAX(int_col) FROM data)` are valid in expression position and smelt already has:

- **Parser support**: `SUBQUERY` syntax kind in `crates/smelt-parser/src/syntax_kind.rs`, parsed via `parse_subquery()` in `parser.rs`, with AST node `Subquery` in `ast.rs` including `Expr::as_subquery()`.
- **Type inference**: `infer_subquery_type()` in `crates/smelt-db/src/type_inference.rs` infers the type from the first column in the subquery's SELECT list and marks the result as always nullable.

What is missing is property test coverage: no generated queries exercise scalar subqueries, so type inference for this path is untested by the randomized tests.

## Key Files

| File | Role |
|------|------|
| `crates/smelt-db/tests/prop_helpers/generators.rs` | Expression generators and query assembly |
| `crates/smelt-db/tests/type_property_tests.rs` | Property test harness that drives generators |
| `crates/smelt-db/src/type_inference.rs` | Type inference including `infer_subquery_type()` |
| `crates/smelt-parser/src/parser.rs` | Parser for `(SELECT ...)` syntax |
| `crates/smelt-parser/src/ast.rs` | `Subquery` AST node, `Expr::as_subquery()` |
| `crates/smelt-db/tests/prop_helpers/type_comparison.rs` | Type comparison logic (may need nullable handling) |
| `crates/smelt-db/tests/prop_helpers/divergences.rs` | Known divergence registry |

## Implementation Steps

### 1. Add `ScalarSubquery` to `ExprKind`

In `generators.rs`, add a new variant to the `ExprKind` enum:

```rust
pub enum ExprKind {
    // ... existing variants ...
    /// Scalar subquery: (SELECT agg(...) FROM data)
    ScalarSubquery,
}
```

Add it to `expr_kind_strategy()` with weight 1 (it is a less common expression form):

```rust
pub fn expr_kind_strategy() -> impl Strategy<Value = ExprKind> {
    prop_oneof![
        // ... existing weights ...
        1 => Just(ExprKind::ScalarSubquery),
    ]
}
```

### 2. Implement `generate_scalar_subquery_expr()`

Add a new generation function. The key constraint is that a scalar subquery must return exactly one row and one column. The simplest way to guarantee one row is to use an aggregate function without GROUP BY. The generated SQL pattern is:

```sql
(SELECT AGG(col) FROM data)
```

Where `AGG` is one of: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`.

Implementation approach:

```rust
fn generate_scalar_subquery_expr(
    columns: &[TypedSource],
    expr_idx: usize,
    func_idx: usize,
    alias: String,
) -> Option<TypedExpr> {
    // Pick an aggregate function and compatible column
    // COUNT works on any type, SUM/AVG need numeric, MIN/MAX work on any
    // Use func_idx to select which aggregate
    // The subquery references the same CTE ("data") that the outer query uses
    // Return type = aggregate's return type, always nullable
}
```

Aggregate choices and their return types:
- `COUNT(col)` -> `BigInt` (always)
- `SUM(col)` -> `Decimal { precision: 38, scale: 10 }` (numeric cols only)
- `AVG(col)` -> `Double` (numeric cols only)
- `MIN(col)` -> same as input type (any col)
- `MAX(col)` -> same as input type (any col)

The function should:
1. Build a small list of candidate aggregates based on available column types.
2. Use `func_idx` to deterministically pick one.
3. Format as `(SELECT AGG(col_name) FROM data)`.
4. Set `expected_smelt_type` to the aggregate's return type.

### 3. Wire into `generate_expr()`

Add the `ScalarSubquery` match arm in `generate_expr()`:

```rust
ExprKind::ScalarSubquery => {
    generate_scalar_subquery_expr(columns, expr_idx, func_idx, alias)
}
```

### 4. Handle scalar subqueries in `assemble_cte_query()`

Scalar subqueries contain their own `FROM data`, so they are self-contained within the outer SELECT. They are not aggregate expressions from the outer query's perspective (the aggregate is inside the subquery). They are also not window expressions.

However, `is_aggregate_expr()` does a simple string check for aggregate function names followed by `(`. A scalar subquery like `(SELECT COUNT(x) FROM data)` starts with `(SELECT`, not with `COUNT(`, so it will not be detected as an aggregate. This means scalar subqueries will be classified as scalar expressions in `assemble_cte_query()`, which is correct -- they can coexist with other scalar and window expressions.

No changes needed to `assemble_cte_query()`.

### 5. Handle nullable comparison

Smelt's `infer_subquery_type()` always marks scalar subquery results as `nullable: true`. DuckDB will also return nullable for aggregates over potentially-empty tables. The type comparison in `type_comparison.rs` compares `DataType` only (not nullability), so this should not cause issues.

Verify that the existing `compare_types()` function handles this correctly -- if it does compare nullability, we may need to account for the always-nullable semantics.

### 6. Add unit tests in generators

Add a unit test in the `mod tests` block at the bottom of `generators.rs`:

```rust
#[test]
fn generate_scalar_subquery() {
    let cols = vec![TypedSource {
        name: "int_col_0".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let expr = generate_expr(&cols, ExprKind::ScalarSubquery, 0, 0).unwrap();
    assert!(expr.sql.starts_with("(SELECT "));
    assert!(expr.sql.contains("FROM data"));
    assert!(expr.sql.ends_with(")"));
}
```

### 7. Run tests and fix divergences

```bash
# Run property tests with default 256 cases
cargo test -p smelt-db --test type_property_tests

# Run with deeper coverage
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
```

If any type mismatches appear:
1. Check if they are known divergences (e.g., SUM returning HUGEINT in DuckDB vs Decimal in smelt) -- add to `divergences.rs`.
2. Check if types are compatible (e.g., Decimal precision differences) -- verify `type_comparison.rs` handles it.
3. If it is a genuine inference bug, fix in `type_inference.rs`.

Likely divergence: DuckDB's `SUM` on integers returns `HUGEINT` (mapped to `Decimal { precision: 38, scale: 0 }` or `BigInt`), while smelt infers `Decimal { precision: 38, scale: 10 }`. This divergence likely already exists for direct SUM expressions and may already be registered. Scalar subqueries wrapping SUM will surface the same divergence.

## Scope and Non-Goals

**In scope:**
- Add `ScalarSubquery` variant to generators
- Generate `(SELECT AGG(col) FROM data)` patterns
- Verify type inference matches DuckDB
- Register any new divergences

**Not in scope (future work):**
- Correlated scalar subqueries (reference outer columns) -- these require more complex context tracking
- Scalar subqueries with WHERE clauses
- Scalar subqueries with their own CTEs (WITH inside the subquery)
- EXISTS subqueries (different return type: always Boolean)
- IN subqueries (already partially covered by InList)
- Nested scalar subqueries (subquery within subquery)

## Verification

1. `cargo fmt --all` passes
2. `cargo clippy --all-targets` passes with no warnings
3. `cargo test` passes (all crates)
4. `cargo test -p smelt-db --test type_property_tests` passes with 256 cases
5. Manual inspection: at least some generated queries contain `(SELECT ... FROM data)` patterns
