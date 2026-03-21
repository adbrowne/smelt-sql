# Plan: GROUP BY / HAVING Property Test Generators

**Date:** 2026-03-22
**Status:** Proposed
**TODO item:** Generate multi-column GROUP BY with HAVING predicates in property test generators.

## Context

The property test generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently handle aggregate functions (COUNT, SUM, AVG, etc.) by emitting them without a GROUP BY clause. This works because DuckDB allows aggregates over an entire table without GROUP BY (producing a single-row result). However, it means we never test:

1. Multi-column GROUP BY with non-aggregate columns in the SELECT list
2. HAVING clauses that filter groups based on aggregate predicates
3. Type inference for HAVING clause expressions (which must be boolean-typed aggregate predicates)

The parser (`smelt-parser`) already fully supports GROUP BY and HAVING syntax (see `GroupByClause` and `HavingClause` in `ast.rs`, parsed in `parser.rs`). Type inference in `smelt-db` already collects column refs from both GROUP BY and HAVING clauses (`lib.rs` lines 1379-1391). What is missing is generator coverage to exercise these code paths through property tests.

## Key Files

- **`crates/smelt-db/tests/prop_helpers/generators.rs`** -- Main file to modify. Contains `ExprKind`, `generate_expr`, `assemble_cte_query`, `test_scenario_strategy`, and supporting types.
- **`crates/smelt-db/tests/type_property_tests.rs`** -- The property test harness that calls into generators. May need minor updates to pass GROUP BY context.
- **`crates/smelt-db/tests/prop_helpers/divergences.rs`** -- May need new divergence entries if DuckDB behavior differs from smelt inference in edge cases.
- **`crates/smelt-db/tests/prop_helpers/type_comparison.rs`** -- Type comparison logic; unlikely to need changes but check for edge cases.
- **`crates/smelt-parser/src/ast.rs`** -- `GroupByClause`, `HavingClause` AST types (read-only reference).
- **`crates/smelt-db/src/type_inference.rs`** -- `infer_select_column_types` and `infer_expression_type` (read-only reference to verify HAVING type inference exists).

## Implementation Steps

### Step 1: Introduce a `QueryShape` enum

Currently `assemble_cte_query` decides query shape (scalar vs aggregate vs window) by inspecting generated expressions. Replace this implicit logic with an explicit `QueryShape` enum:

```rust
pub enum QueryShape {
    /// SELECT scalar_exprs FROM data
    Scalar,
    /// SELECT group_cols, agg_exprs FROM data GROUP BY group_cols
    GroupBy {
        group_columns: Vec<String>,  // column names to GROUP BY
    },
    /// SELECT group_cols, agg_exprs FROM data GROUP BY group_cols HAVING predicate
    GroupByHaving {
        group_columns: Vec<String>,
        having_predicate: String,    // e.g. "COUNT(*) > 0"
        having_expected_type: DataType, // always Boolean
    },
    /// SELECT window_exprs FROM data
    Window,
}
```

The existing `assemble_cte_query` logic already separates aggregate-only queries from mixed queries. This enum makes the intent explicit and adds the GROUP BY / HAVING variants.

### Step 2: Add GROUP BY assembly to `assemble_cte_query`

Modify `assemble_cte_query` (or create a new `assemble_query` that takes a `QueryShape`) to append `GROUP BY` and `HAVING` clauses:

```
WITH data AS (SELECT ...)
SELECT group_col_0, group_col_1, COUNT(agg_col) AS expr_0, SUM(agg_col) AS expr_1
FROM data
GROUP BY group_col_0, group_col_1
HAVING COUNT(agg_col) > 0
```

Key constraints:
- Non-aggregate expressions in the SELECT list must appear in the GROUP BY clause
- The HAVING predicate must be a boolean expression involving aggregates
- GROUP BY columns retain their original types; aggregate expressions have their function-determined types

### Step 3: Generate HAVING predicates

Create a `generate_having_predicate` function that produces boolean-valued aggregate predicates:

- `COUNT(*) > 0` (always valid, returns Boolean)
- `COUNT(col) > 0` (valid for any column)
- `SUM(numeric_col) > 0` (requires numeric GROUP BY input)
- `AVG(numeric_col) > 0` (requires numeric)
- `MIN(col) IS NOT NULL` (valid for any type)
- `MAX(col) IS NOT NULL` (valid for any type)

The predicate itself does not appear in the SELECT list, so it does not directly affect type inference of output columns. However, it exercises the parser's HAVING clause and the type inference's column-ref collection from HAVING expressions.

### Step 4: Add `GroupBy` expression kind or strategy variant

Two options (prefer Option B):

**Option A:** Add `ExprKind::GroupByAgg` variant. When this kind is selected, `generate_expr` returns an aggregate expression and marks it as needing GROUP BY context. Then `assemble_cte_query` detects these and adds GROUP BY for the remaining non-aggregate columns.

**Option B:** Add a separate `query_shape_strategy()` that sometimes produces GROUP BY queries. This is cleaner because it separates the "what expressions to generate" concern from the "what query shape to use" concern.

Implementation for Option B:
1. Add `query_shape_strategy() -> impl Strategy<Value = QueryShape>` that picks Scalar (60%), GroupBy (20%), GroupByHaving (15%), Window (5%)
2. When GroupBy or GroupByHaving is selected, pick 1-3 columns from the pool as GROUP BY columns
3. Generate aggregate expressions on the remaining columns
4. For GroupByHaving, generate a HAVING predicate using `generate_having_predicate`
5. Update `test_scenario_strategy` to include the `QueryShape` in its output tuple
6. Update `assemble_cte_query` to accept the `QueryShape` and build the query accordingly

### Step 5: Update `test_scenario_strategy` and test harness

Modify `test_scenario_strategy` to return `(Vec<TypedSource>, QueryShape, Vec<ExprKind>, Vec<usize>)`. The property test in `type_property_tests.rs` needs to:

1. Accept the new `QueryShape` from the strategy
2. When `QueryShape::GroupBy` or `QueryShape::GroupByHaving`, generate aggregate expressions only for the non-group columns, and include group columns as plain column refs in the SELECT list
3. Pass the `QueryShape` to the updated `assemble_cte_query`

### Step 6: Add deterministic smoke tests

Add unit tests in the `tests` module at the bottom of `generators.rs`:

```rust
#[test]
fn assemble_group_by_query() {
    // Verify GROUP BY clause is appended correctly
}

#[test]
fn assemble_group_by_having_query() {
    // Verify HAVING clause is appended after GROUP BY
}

#[test]
fn group_by_type_inference_matches_duckdb() {
    // End-to-end: generate a GROUP BY query, run against DuckDB,
    // verify smelt infers the same types
}
```

### Step 7: Handle edge cases

- **Single-column GROUP BY**: Must work when the column pool has only 1 column. In this case, GROUP BY that column and SELECT only aggregates plus that column.
- **All columns in GROUP BY**: If all columns are selected for GROUP BY, there are no columns left for aggregates. Use `COUNT(*)` as the aggregate expression.
- **Decimal aggregates**: SUM on DECIMAL columns returns DECIMAL(38,s) in DuckDB; check if this matches smelt inference or needs a divergence entry.
- **Boolean in GROUP BY**: Valid in SQL; DuckDB supports it. Ensure the generator can pick boolean columns for GROUP BY.

## Verification

1. `cargo test -p smelt-db --test type_property_tests` -- All property tests pass (including new GROUP BY variants)
2. `cargo test -p smelt-db --test type_property_tests -- group_by` -- Smoke tests pass
3. `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` -- Extended run to catch edge cases
4. `cargo clippy --all-targets` -- No warnings
5. `cargo fmt --all -- --check` -- Formatting clean

## Risks and Mitigations

- **SUM type divergence**: DuckDB returns `DECIMAL(38,s)` for SUM on integers while smelt may infer differently. Mitigation: check existing `function_return_type` for SUM (currently returns `Decimal { precision: 38, scale: 10 }`). May need adjustment or a divergence entry since DuckDB preserves the input scale.
- **HAVING with complex predicates**: Compound HAVING predicates (AND/OR) could trigger parser edge cases. Mitigation: start with simple single-predicate HAVING (e.g., `COUNT(*) > 0`) and expand later.
- **Strategy balance**: Adding GROUP BY queries reduces coverage of scalar/window queries. Mitigation: keep GROUP BY probability moderate (20% GroupBy + 15% Having = 35% total).
