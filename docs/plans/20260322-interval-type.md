# Plan: Interval Type for Property Test Generators

**Date:** 2026-03-22
**Status:** Proposed
**TODO item:** "Interval type -- Add as a base type for temporal arithmetic testing"

## Context

The property test generators (`crates/smelt-db/tests/prop_helpers/generators.rs`) define a `BaseType` enum that controls which column types appear in randomly generated CTE queries. Currently there are 8 base types: Boolean, Integer, BigInt, Double, Varchar, Date, Timestamp, and Decimal.

Interval is already defined in the `DataType` enum (`crates/smelt-types/src/lib.rs:60`) and the Arrow mapping already converts `ArrowType::Duration` and `ArrowType::Interval` to `DataType::Interval` (`arrow_mapping.rs:42`). The type inference also returns `DataType::Interval` for the `AGE()` function. However, Interval is **not** available as a base column type in the generators, which means temporal arithmetic expressions like `date_col + INTERVAL '1 day'` or `ts_col - INTERVAL '3 hours'` are never tested.

Adding Interval as a base type enables:
1. Testing interval literal generation (`INTERVAL '1 day'`, `INTERVAL '3 hours'`)
2. Testing temporal arithmetic (`date + interval -> date`, `timestamp - interval -> timestamp`, `date - date -> interval`)
3. Testing interval-specific functions (`AGE()`, `DATE_PART` on intervals)
4. Verifying smelt's type inference matches DuckDB for interval operations

## Key Files to Modify

1. **`crates/smelt-db/tests/prop_helpers/generators.rs`** -- Add `Interval` to `BaseType` enum and expression generators
2. **`crates/smelt-db/src/type_inference.rs`** -- Add temporal arithmetic type rules to `infer_binary_expr_type`
3. **`crates/smelt-db/tests/prop_helpers/arrow_mapping.rs`** -- Already handles Interval (no changes needed)
4. **`crates/smelt-db/tests/prop_helpers/divergences.rs`** -- Possible new divergences for interval operations
5. **`crates/smelt-types/src/lib.rs`** -- Already has `DataType::Interval` (no changes needed)

## Implementation Steps

### Step 1: Add `BaseType::Interval` to generators

In `generators.rs`, extend the `BaseType` enum and its methods:

- Add `Interval` variant to the `BaseType` enum
- Add `BaseType::Interval` to the `all()` array
- Add `to_smelt_type`: `BaseType::Interval => DataType::Interval`
- Add `cast_sql`: `BaseType::Interval => "CAST('1 day' AS INTERVAL)"` (DuckDB supports this syntax)
- Add `col_prefix`: `BaseType::Interval => "interval_col"`

### Step 2: Add `FuncInput::Interval` discriminant (optional)

Consider adding a `FuncInput::Interval` variant (or reusing `Temporal`) for functions that specifically require interval arguments. Functions like `DATE_PART('day', interval_col)` accept intervals. The simpler approach is to group intervals under `FuncInput::Temporal` and handle them in the matching logic.

### Step 3: Extend `BinaryOp` generation for temporal arithmetic

The `BinaryOp` arm in `generate_expr` currently only handles numeric `+` and string `||`. Extend it to generate temporal arithmetic when interval and date/timestamp columns are available:

- `date_col + interval_col` -> expected type: `DataType::Date` (in DuckDB, actually returns Timestamp)
- `ts_col + interval_col` -> expected type: `DataType::Timestamp { with_timezone: false }`
- `date_col - interval_col` -> expected type: `DataType::Date` (in DuckDB, returns Timestamp)
- `ts_col - interval_col` -> expected type: `DataType::Timestamp { with_timezone: false }`
- `date_col - date_col` -> expected type: `DataType::Interval` (in DuckDB, returns BigInt representing days)
- `ts_col - ts_col` -> expected type: `DataType::Interval`
- `interval_col + interval_col` -> expected type: `DataType::Interval`
- `interval_col * int_col` -> expected type: `DataType::Interval`

**Important DuckDB behavior note:** DuckDB's `DATE + INTERVAL` returns a `TIMESTAMP`, not a `DATE`. This will likely need a divergence entry or the inference should match DuckDB's behavior. Similarly, `DATE - DATE` returns a `BIGINT` (number of days) in DuckDB, not an `INTERVAL`. These need careful handling.

### Step 4: Extend `infer_binary_expr_type` in type inference

Add temporal arithmetic rules to the `"+"` and `"-"` match arms in `infer_binary_expr_type`:

For `+`:
```
(Date|Timestamp, Interval) | (Interval, Date|Timestamp) => Timestamp
(Interval, Interval) => Interval
```

For `-`:
```
(Date|Timestamp, Interval) => Timestamp
(Date, Date) => Interval  (note: DuckDB returns BigInt)
(Timestamp, Timestamp) => Interval
(Interval, Interval) => Interval
```

For `*`:
```
(Interval, Integer|BigInt) | (Integer|BigInt, Interval) => Interval
```

These rules should be checked **before** the numeric promotion cascade so that temporal types are not accidentally matched by the generic fallback.

### Step 5: Register expected DuckDB divergences

Based on DuckDB's actual behavior, register divergences in `divergences.rs`:

- `date_plus_interval`: smelt infers `Date`, DuckDB returns `Timestamp` -- **or** change smelt to infer `Timestamp` to match DuckDB
- `date_minus_date`: smelt infers `Interval`, DuckDB returns `BigInt` (days count)

The better approach is probably to have smelt match DuckDB's behavior (return Timestamp for date +/- interval) since that is what users will observe. The `date - date -> BigInt` case is trickier and may warrant a `BackendSpecific` divergence since PostgreSQL returns `Interval` for that expression.

### Step 6: Add interval-aware function generators

Add interval-relevant functions to `core_functions()`:

- `DATE_PART('day', interval_col)` -- already has `FuncInput::Temporal`, but the matching logic in `generate_function_expr` needs to also match `DataType::Interval` as a temporal type
- `AGE(ts_col, ts_col)` -- returns `Interval`, but requires two timestamp arguments (complex generator logic, can defer)

### Step 7: Update `is_temporal` helper (if exists) or matching logic

Check if the function matching uses `is_temporal()` or similar helpers. Ensure `DataType::Interval` is included in temporal type checks where appropriate, but excluded where it does not apply (e.g., `DATE_TRUNC` does not accept intervals).

## Testing and Verification

1. **Run existing tests first**: `cargo test -p smelt-db --test type_property_tests` to confirm no regressions
2. **Run with higher case count**: `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` to exercise interval expressions
3. **Manual smoke test**: Verify DuckDB behavior for key expressions:
   ```sql
   SELECT TYPEOF(DATE '2024-01-01' + INTERVAL '1 day');       -- TIMESTAMP
   SELECT TYPEOF(TIMESTAMP '2024-01-01' - INTERVAL '1 hour'); -- TIMESTAMP
   SELECT TYPEOF(DATE '2024-01-02' - DATE '2024-01-01');      -- BIGINT (not INTERVAL!)
   SELECT TYPEOF(INTERVAL '1 day' + INTERVAL '2 days');       -- INTERVAL
   SELECT TYPEOF(INTERVAL '1 day' * 3);                       -- INTERVAL
   ```
4. **Check clippy and fmt**: `cargo clippy --all-targets && cargo fmt --all -- --check`

## Open Questions

1. **Should smelt infer `Timestamp` for `Date + Interval`?** DuckDB does this, but PostgreSQL returns `Timestamp` too, so this is actually consistent across backends. Smelt should probably follow this convention.

2. **Should `Date - Date` return `Interval` or `BigInt`?** DuckDB returns `BigInt` (days), PostgreSQL returns `Interval`. This is a genuine backend divergence. Options:
   - Infer `Interval` (semantically correct) and register DuckDB divergence
   - Infer `BigInt` to match DuckDB (pragmatic but PostgreSQL-incompatible)
   - Infer `Interval` with a note that DuckDB silently converts

3. **Interval sub-types?** DuckDB has year-month and day-time interval components. For now, treat Interval as a single opaque type (matching the current `DataType::Interval` definition). Sub-type distinctions can be added later if needed.

## Estimated Scope

- **Small-medium change**: ~150-250 lines across generators.rs and type_inference.rs
- **Risk**: Low -- additive change, existing tests unaffected
- **Dependencies**: None -- all infrastructure (DataType::Interval, Arrow mapping) already exists
