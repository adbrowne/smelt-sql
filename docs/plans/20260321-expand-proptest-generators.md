# Expand Property Test Generator Coverage

## Context

The property test generators in `crates/smelt-db/tests/prop_helpers/generators.rs` only cover ~15 of the ~80+ functions supported by the type inference engine. The TODO at `docs/TODO.md` lists all gaps. This plan adds missing coverage incrementally — one commit per batch of related functions — testing and fixing issues as we go.

## Approach

Add functions in order from simplest (no infrastructure changes) to most complex (multi-arg support). Each step: add to generators, run tests, fix mismatches, commit.

## Steps

### Step 1: Single-arg math functions -> Double
Add to `core_functions()`: **EXP, LN, LOG, LOG10, LOG2, POWER, POW, SIN, COS, TAN, ASIN, ACOS, ATAN, SINH, COSH, TANH**
- All use `FuncInput::Numeric`, return `DataType::Double`
- Update `function_return_type()` if needed (most already handled)
- No infrastructure changes

### Step 2: SIGN function
- `FuncInput::Numeric`, arg-dependent return type (like ABS)
- Update `function_return_type()`: SIGN -> arg type

### Step 3: Single-arg string functions
Add: **LTRIM, RTRIM, INITCAP, CONCAT** (single-arg), **CHAR_LENGTH, CHARACTER_LENGTH**
- LTRIM/RTRIM/INITCAP/CONCAT: `FuncInput::String` -> `DataType::Text`
- CHAR_LENGTH/CHARACTER_LENGTH: `FuncInput::String` -> `DataType::BigInt`
- `function_return_type()` already covers these

### Step 4: COALESCE (new FuncInput::AnyScalar)
- Add `FuncInput::AnyScalar` variant (non-aggregate, any type)
- COALESCE with single arg: `FuncInput::AnyScalar`, returns arg type
- Update `is_compatible()`, `function_return_type()`

### Step 5: GREATEST and LEAST
- Use `FuncInput::AnyScalar` from Step 4, return arg type

### Step 6: Statistical aggregates + refactor aggregate detection
Add: **STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP**
- All `FuncInput::NumericAggregate` -> `DataType::Double`
- **Refactor** `assemble_cte_query()`: replace fragile `starts_with("COUNT(")` checks with a helper that extracts the function name and checks `SqlFunction::from_name() + is_aggregate()`
- Update `function_return_type()`

### Step 7: Boolean aggregates (new FuncInput::BooleanAggregate)
Add: **BOOL_AND, BOOL_OR, EVERY**
- New `FuncInput::BooleanAggregate` -> `DataType::Boolean`
- Update `is_compatible()`: only `BaseType::Boolean`

### Step 8: Bit aggregates (new FuncInput::IntegerAggregate)
Add: **BIT_AND, BIT_OR, BIT_XOR**
- New `FuncInput::IntegerAggregate`, arg-dependent return type
- Update `is_compatible()`: only `BaseType::Integer | BaseType::BigInt`

### Step 9: PI() zero-arg function (new FuncInput::NoArg)
- New `FuncInput::NoArg` variant
- Update `generate_expr()`: emit `PI()` without column reference
- Update `is_compatible()`: always true for NoArg

### Step 10: Multi-arg infrastructure + functions
**Infrastructure**: Add `extra_args: &'static [ExtraArg]` to `FuncDesc`:
```rust
enum ExtraArg {
    SameAsFirst,           // re-use the same column
    IntLiteral(&'static str),
    StringLiteral(&'static str),
}
```
Update `generate_expr()` to assemble multi-arg calls.

**Functions**: REPLACE(str,str,str), LPAD(str,int,str), RPAD(str,int,str), LEFT(str,int), RIGHT(str,int), REPEAT(str,int), NULLIF(any,any), POWER->multi-arg, MOD(num,num), ATAN2(num,num)

### Step 11: SUBSTRING, SPLIT_PART, STRPOS
- SUBSTRING(str,int,int), SUBSTR(str,int,int), SPLIT_PART(str,str,int), STRPOS(str,str)
- Uses multi-arg infrastructure from Step 10
- Skip POSITION (special syntax); STRPOS covers same inference path

### Step 12: DATE_PART and DATE_TRUNC
- Need literal-first arg order: `DATE_PART('year', col)`, `DATE_TRUNC('month', col)`
- Add `prepend_literal: Option<&'static str>` to `FuncDesc` (or similar)
- DATE_PART -> BigInt, DATE_TRUNC -> Timestamp
- Skip EXTRACT (special syntax); DATE_PART covers same code path

### Step 13: Expanded CAST targets + BETWEEN/IN expressions
- Expand `ExprKind::Cast` to target Date, Timestamp, Boolean, Integer, BigInt, Decimal
- Add `ExprKind::Between` and `ExprKind::InList` -> Boolean
- Update `expr_kind_strategy()` with low probability for new variants

## Key Files

- `crates/smelt-db/tests/prop_helpers/generators.rs` — all generator changes
- `crates/smelt-db/src/type_inference.rs` — reference for expected types
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — register new divergences
- `crates/smelt-db/tests/prop_helpers/type_comparison.rs` — if new compat pairs needed
- `docs/TODO.md` — check off completed items

## Verification

After each step:
```bash
cargo test -p smelt-db --test type_property_tests
cargo clippy --all-targets
cargo fmt --all -- --check
```

At end: push branch, open PR.
