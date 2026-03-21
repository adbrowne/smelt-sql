# Plan: GROUP BY + Window Functions Combined Generators

**Date:** 2026-03-22
**Status:** Proposed
**TODO item:** "GROUP BY + window functions combined" in `docs/TODO.md`

## Context

The property test generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently separate aggregate and window expressions into different queries. When both are present, `assemble_cte_query` drops the aggregates and keeps only window + scalar expressions (lines 1164-1170). This means we never test queries that combine GROUP BY with window functions, which is a common and important SQL pattern:

```sql
-- Window over aggregated results
SELECT category, SUM(amount) AS total,
       RANK() OVER (ORDER BY SUM(amount)) AS rank_by_total
FROM data
GROUP BY category

-- Window alongside GROUP BY columns
SELECT category, COUNT(*) AS cnt,
       ROW_NUMBER() OVER (ORDER BY category) AS rn
FROM data
GROUP BY category
```

These patterns exercise type inference paths where:
1. Aggregate return types feed into window function ORDER BY / PARTITION BY
2. GROUP BY columns appear alongside both aggregates and window functions
3. The query has both GROUP BY and OVER clauses simultaneously

## Key Files

- `crates/smelt-db/tests/prop_helpers/generators.rs` -- Main file to modify (expression generators, query assembly)
- `crates/smelt-db/tests/type_property_tests.rs` -- Property test harness (may need minor updates for new query shapes)
- `crates/smelt-db/src/type_inference.rs` -- Type inference implementation (target of testing, should not need changes unless bugs found)
- `crates/smelt-db/tests/prop_helpers/divergences.rs` -- May need new entries if DuckDB/smelt disagree on combined query types

## Implementation Steps

### Step 1: Add a new query mode enum

Add a `QueryMode` enum to represent the three query shapes the generator can produce:

```rust
enum QueryMode {
    /// Plain SELECT without GROUP BY (current default for scalar/window)
    Plain,
    /// SELECT with GROUP BY, aggregates only (current aggregate path)
    AggregateOnly,
    /// SELECT with GROUP BY + window functions over aggregated/grouped columns
    GroupByWithWindow,
}
```

### Step 2: Add a GROUP BY + window expression generator

Create `generate_groupby_window_scenario` that builds a complete set of expressions for a combined query. This function takes the column pool and returns a tuple of `(group_by_columns, select_expressions)`:

1. **Pick 1-2 GROUP BY columns** from the column pool (preferring non-numeric columns like VARCHAR, DATE for realistic grouping, but any type works).
2. **Generate 1-2 aggregate expressions** (COUNT, SUM, AVG, etc.) over non-grouped columns, using the existing `FuncDesc` table filtered to `FuncInput::*Aggregate` variants.
3. **Generate 1-2 window expressions** where:
   - The ORDER BY inside OVER references an aggregate expression (e.g., `RANK() OVER (ORDER BY SUM(x))`) -- this is the key new pattern.
   - Alternatively, the PARTITION BY references a GROUP BY column and ORDER BY references another GROUP BY column or aggregate.
4. **Include GROUP BY columns** as plain column refs in the SELECT list.

The key insight: in a GROUP BY query, window functions can reference both GROUP BY columns and aggregate expressions in their OVER clause. The window is computed *after* the GROUP BY aggregation.

### Step 3: Modify `assemble_cte_query` to emit GROUP BY

Extend the function signature or create a new assembly function `assemble_grouped_cte_query`:

```rust
pub fn assemble_grouped_cte_query(
    columns: &[TypedSource],
    group_by_cols: &[String],       // column names for GROUP BY
    select_exprs: &[TypedExpr],     // mix of col refs, aggregates, and window funcs
) -> String
```

This emits SQL like:
```sql
WITH data AS (SELECT CAST(42 AS INTEGER) AS x, CAST('a' AS VARCHAR) AS cat)
SELECT cat, SUM(x) AS expr_0, RANK() OVER (ORDER BY SUM(x)) AS expr_1
FROM data
GROUP BY cat
```

The existing `assemble_cte_query` remains unchanged for backward compatibility.

### Step 4: Add a combined test scenario strategy

Create `grouped_window_scenario_strategy` as a new proptest strategy:

```rust
pub fn grouped_window_scenario_strategy()
    -> impl Strategy<Value = GroupedWindowScenario>
```

Where `GroupedWindowScenario` contains:
- `columns: Vec<TypedSource>` -- the CTE column pool
- `group_by_cols: Vec<String>` -- which columns to GROUP BY
- `select_exprs: Vec<TypedExpr>` -- the full SELECT list with expected types

This strategy:
1. Generates a column pool of 2-5 columns (need at least 2: one for grouping, one for aggregating).
2. Partitions columns into group-by set and aggregate-target set.
3. Generates aggregate expressions over the aggregate targets.
4. Generates window expressions that reference aggregates in OVER clauses.
5. Combines group-by column refs + aggregates + window exprs into the SELECT list.

### Step 5: Generate window-over-aggregate expressions

The most important new pattern. Create a helper `generate_window_over_aggregate_expr`:

```rust
fn generate_window_over_aggregate_expr(
    group_by_cols: &[&TypedSource],
    agg_exprs: &[(String, DataType)],  // (sql, return_type) of generated aggregates
    func_idx: usize,
    alias: String,
) -> TypedExpr
```

This picks a window function (e.g., RANK, ROW_NUMBER, DENSE_RANK) and builds an OVER clause where:
- `ORDER BY` uses an aggregate expression SQL snippet (e.g., `SUM(x)`)
- `PARTITION BY` optionally uses a GROUP BY column

Return type inference follows the same rules as existing `generate_window_expr` -- it depends on the window function kind, not the ORDER BY expression.

### Step 6: Wire into the property test

In `type_property_tests.rs`, add a new proptest (or extend the existing one) that uses `grouped_window_scenario_strategy`:

```rust
proptest! {
    #[test]
    fn prop_grouped_window_type_inference(
        scenario in grouped_window_scenario_strategy()
    ) {
        let sql = assemble_grouped_cte_query(
            &scenario.columns,
            &scenario.group_by_cols,
            &scenario.select_exprs,
        );
        // ... run smelt inference + DuckDB oracle comparison
    }
}
```

Alternatively, integrate into the existing `prop_type_inference` test by extending `test_scenario_strategy` to sometimes produce grouped-window scenarios. This avoids a separate test function and exercises the same comparison logic.

### Step 7: Handle edge cases

- **Single-row CTE**: The CTE only has one row, so GROUP BY always produces one group. This is fine for type checking purposes -- we only care about types, not values.
- **Duplicate aliases**: Ensure generated aliases don't collide between aggregate exprs and window exprs (use `expr_0`, `expr_1`, etc. sequentially).
- **Empty aggregate set**: If no aggregate-compatible columns exist (e.g., all columns are DATE), fall back to COUNT(*) which works on any input.
- **Window function that needs ORDER BY**: Most ranking functions require ORDER BY. When ordering by an aggregate, use the raw aggregate expression (e.g., `ORDER BY SUM(x)`), not the alias.

## Testing and Verification

1. **Run existing tests first** to ensure no regressions: `cargo test -p smelt-db --test type_property_tests`
2. **Run the new property test** with default cases (256): `cargo test -p smelt-db --test type_property_tests prop_grouped_window` (or the extended existing test)
3. **Run with higher coverage**: `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests`
4. **Manual spot-check**: Add a unit test in `generators.rs` `mod tests` that verifies `assemble_grouped_cte_query` produces valid SQL for a hand-crafted scenario:
   ```rust
   #[test]
   fn assemble_grouped_window_query() {
       // Verify: SELECT cat, SUM(x) AS expr_0, RANK() OVER (ORDER BY SUM(x)) AS expr_1
       //         FROM data GROUP BY cat
   }
   ```
5. **Clippy + fmt**: `cargo clippy --all-targets && cargo fmt --all`

## Risks and Mitigations

- **DuckDB type divergences**: Combined queries may surface new type disagreements between smelt inference and DuckDB. Mitigation: add any legitimate divergences to `divergences.rs`.
- **Complexity in generator**: The grouped-window generator is more complex than existing generators because it must coordinate multiple expression categories. Mitigation: keep the generator deterministic given its inputs (column pool + indices), and use helper functions for each sub-generation step.
- **Parser coverage**: The smelt parser must correctly handle GROUP BY + window function queries. This is already supported (GROUP BY is parsed, window OVER clauses are parsed). The property test will confirm type inference works end-to-end.
