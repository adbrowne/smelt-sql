# Plan: Mixed-type binary operation generators

**Date**: 2026-03-22
**Status**: Proposed
**TODO item**: Mixed-type binary operations -- Generate cross-type arithmetic (INTEGER + BIGINT, DECIMAL + DOUBLE) to test type promotion rules.

## Context

The property test generators in `generators.rs` currently only produce same-type binary operations. The `ExprKind::BinaryOp` arm finds a single numeric column and generates `col + col`, which always tests `T + T -> T`. This misses an important class of expressions: cross-type arithmetic like `INTEGER + BIGINT` or `DECIMAL * DOUBLE`, where the result type depends on promotion rules.

smelt already implements a full numeric promotion hierarchy in `type_inference.rs`:

```
SmallInt < Integer < BigInt < Decimal < Double
```

But this logic is never exercised by the property tests. If the promotion rules diverge from DuckDB's actual behavior, we would not detect it.

## Goal

Generate binary expressions that combine two *different* numeric types, predict the promoted result type using smelt's rules, and verify it matches DuckDB.

## Key files

| File | Role |
|------|------|
| `crates/smelt-db/tests/prop_helpers/generators.rs` | Expression generators -- main file to modify |
| `crates/smelt-db/src/type_inference.rs` | `infer_binary_expr_type` and `promote_types` -- reference for expected promotion rules |
| `crates/smelt-db/tests/prop_helpers/type_comparison.rs` | Type comparison logic -- may need updates for new compatible pairs |
| `crates/smelt-db/tests/prop_helpers/divergences.rs` | Known divergences -- may need new entries if DuckDB differs |
| `crates/smelt-db/tests/type_property_tests.rs` | The property test harness that drives generation and verification |

## Implementation steps

### 1. Add a `MixedBinaryOp` variant to `ExprKind`

In `generators.rs`, add a new variant:

```rust
enum ExprKind {
    // ... existing variants ...
    /// Cross-type binary arithmetic (e.g., INTEGER + BIGINT).
    MixedBinaryOp,
}
```

Include it in the proptest strategy so it gets selected with reasonable frequency.

### 2. Define the numeric type pairs and their expected promotions

Create a lookup table of cross-type pairs and their expected result types, mirroring the promotion hierarchy from `infer_binary_expr_type`:

| Left | Right | Expected Result |
|------|-------|-----------------|
| Integer | BigInt | BigInt |
| Integer | Double | Double |
| Integer | Decimal(10,2) | Decimal(38,10) |
| BigInt | Double | Double |
| BigInt | Decimal(10,2) | Decimal(38,10) |
| Decimal(10,2) | Double | Double |
| SmallInt | Integer | Integer |
| SmallInt | BigInt | BigInt |
| SmallInt | Double | Double |
| SmallInt | Decimal(10,2) | Decimal(38,10) |

Note: SmallInt is not currently a `BaseType` in the generator. We can either add it or limit to the existing base types (Integer, BigInt, Double, Decimal). Starting with the existing four is simpler and covers the most important promotion paths.

### 3. Implement the `MixedBinaryOp` generator arm

In the `match expr_kind` block of `generate_typed_expr`, add:

```rust
ExprKind::MixedBinaryOp => {
    // Find two numeric columns of *different* types
    let numeric_cols: Vec<_> = columns.iter()
        .filter(|c| c.data_type.is_numeric())
        .collect();

    // Need at least two different numeric types
    let pairs: Vec<_> = numeric_cols.iter()
        .flat_map(|a| numeric_cols.iter().map(move |b| (a, b)))
        .filter(|(a, b)| std::mem::discriminant(&a.data_type) != std::mem::discriminant(&b.data_type))
        .collect();

    if pairs.is_empty() {
        return None; // Fall back; no cross-type pair available
    }

    let (left, right) = pairs[expr_idx % pairs.len()];
    let op = ["+", "-", "*"][func_idx % 3];
    let expected = predict_promotion(&left.data_type, &right.data_type, op);

    Some(TypedExpr {
        sql: format!("{} {} {}", left.name, op, right.name),
        alias,
        expected_smelt_type: expected,
    })
}
```

### 4. Add a `predict_promotion` helper

This function encodes the same logic as `infer_binary_expr_type` for arithmetic operators:

```rust
fn predict_promotion(left: &DataType, right: &DataType, op: &str) -> DataType {
    match (left, right) {
        (DataType::Double, _) | (_, DataType::Double) => DataType::Double,
        (DataType::Decimal { .. }, _) | (_, DataType::Decimal { .. }) => {
            DataType::Decimal { precision: 38, scale: 10 }
        }
        (DataType::BigInt, _) | (_, DataType::BigInt) => DataType::BigInt,
        (DataType::Integer, _) | (_, DataType::Integer) => DataType::Integer,
        (DataType::SmallInt, _) | (_, DataType::SmallInt) => DataType::SmallInt,
        _ => DataType::Unknown,
    }
}
```

Division (`/`) may need special handling: in DuckDB, `INTEGER / INTEGER` returns `INTEGER` (truncating), but `DECIMAL / DECIMAL` may change precision. Start with `+`, `-`, `*` and add `/` as a follow-up if needed.

### 5. Ensure the CTE has multiple numeric column types

The existing `generate_cte_query` already creates one column per `BaseType`, so Integer, BigInt, Double, and Decimal columns are always present. No changes needed here.

### 6. Update type comparison if needed

DuckDB's promotion results might differ from smelt in edge cases:

- **Integer + Decimal**: DuckDB may return a specific `DECIMAL(p,s)` rather than `DECIMAL(38,10)`. The existing `is_decimal_compat` treats any `Decimal <-> Decimal` as compatible, so this should already pass.
- **Integer + BigInt**: Both smelt and DuckDB should return BigInt. No issue expected.
- **Anything + Double**: Both should return Double. No issue expected.

If new mismatches surface, add entries to `divergences.rs` with `BackendSpecific` status.

### 7. Add targeted smoke tests

Add 2-3 deterministic smoke tests in `type_property_tests.rs` for the most important cross-type cases:

```rust
#[test]
fn smoke_integer_plus_bigint() { /* SELECT CAST(1 AS INTEGER) + CAST(2 AS BIGINT) */ }

#[test]
fn smoke_decimal_times_double() { /* SELECT CAST(1.5 AS DECIMAL(10,2)) * CAST(2.0 AS DOUBLE) */ }

#[test]
fn smoke_integer_plus_decimal() { /* SELECT CAST(1 AS INTEGER) + CAST(2.5 AS DECIMAL(10,2)) */ }
```

These provide fast regression coverage without needing the full proptest harness.

## Testing and verification

1. Run the property tests with default case count:
   ```bash
   cargo test -p smelt-db --test type_property_tests
   ```

2. Run with higher case count to stress cross-type paths:
   ```bash
   PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
   ```

3. Verify no new `Mismatch` results appear. Any unexpected mismatches indicate either:
   - A bug in smelt's promotion logic (fix in `type_inference.rs`)
   - A known DuckDB difference (add to `divergences.rs`)
   - A precision difference (already handled by `type_comparison.rs`)

4. Run the full test suite and clippy:
   ```bash
   cargo test && cargo clippy --all-targets && cargo fmt --all -- --check
   ```

## Risks and edge cases

- **Division**: `INTEGER / INTEGER` is integer division in DuckDB but smelt's `infer_binary_expr_type` treats `/` the same as `+` and `*`. This could surface a real divergence. Consider excluding `/` initially or handling it separately.
- **Decimal precision arithmetic**: DuckDB computes exact result precision for decimal operations (e.g., `DECIMAL(10,2) + DECIMAL(5,3)` yields `DECIMAL(11,3)`). smelt uses a fixed `(38,10)` envelope. The existing `is_decimal_compat` handles this, but if DuckDB returns a precision > 38, that would be a new case.
- **Subtraction with unsigned types**: Not relevant for smelt's current type set, but worth noting for future SmallInt/TinyInt additions.

## Scope

This plan covers only arithmetic binary operators (`+`, `-`, `*`) with cross-type numeric operands. It does not cover:
- Comparison operators across types (e.g., `INTEGER < BIGINT`) -- these always return Boolean regardless
- String concatenation with non-string types -- separate concern
- Division operator -- deferred due to integer-division semantics complexity
