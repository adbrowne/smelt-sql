# Multi-Model Property Tests

**Date:** 2026-03-22
**Status:** Proposed

## Context

The existing property tests in `type_property_tests.rs` verify smelt's type inference for single-model queries only. Each test case generates a CTE-based query (e.g., `WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT UPPER(x) AS expr_0 FROM data`) and compares smelt's inferred types against DuckDB's actual types.

However, smelt's core value proposition is cross-model type inference: when model_B does `SELECT col FROM smelt.ref('model_A')`, the types of `col` should be inferred from model_A's output schema and flow through correctly. This cross-model type propagation path is currently untested by property tests.

The Salsa-based `Database` already supports multi-model setups (see existing tests like `test_schema_extraction_from_ref` in `lib.rs`), but the property test harness bypasses Salsa entirely -- it calls `smelt_parser::parse()` and `infer_select_column_types()` directly with a manually constructed `TypeContext`.

Multi-model property tests would exercise the full Salsa query pipeline (`file_text` -> `parse_file` -> `model_schema` -> `type_context` -> `typed_model_schema`) and catch bugs in cross-model type propagation that single-model tests cannot detect.

## Key Files

- **`crates/smelt-db/tests/type_property_tests.rs`** -- Main test file; add new `prop_multi_model_type_inference` test
- **`crates/smelt-db/tests/prop_helpers/generators.rs`** -- Add generators for two-model SQL pairs
- **`crates/smelt-db/tests/prop_helpers/duckdb_oracle.rs`** -- May need a helper for two-query type checking
- **`crates/smelt-db/src/lib.rs`** -- Reference for Salsa `Database` setup (existing tests at line ~1580)
- **`crates/smelt-db/src/type_inference.rs`** -- `TypeContext`, `infer_select_column_types`

## Design

### Test Shape

Each property test case generates a two-model chain:

```
model_A.sql:  SELECT CAST(42 AS INTEGER) AS int_col, CAST('hi' AS VARCHAR) AS str_col
model_B.sql:  SELECT UPPER(str_col) AS expr_0, int_col + 1 AS expr_1 FROM smelt.ref('model_A')
```

The test then:
1. Sets up a Salsa `Database` with both models registered
2. Queries `typed_model_schema(model_B)` to get smelt's inferred types
3. Constructs an equivalent single DuckDB query (flattening the ref into a CTE) and queries DuckDB for actual types
4. Compares the two, using the same `compare_types` / `divergences` infrastructure

### DuckDB Oracle Strategy

DuckDB does not understand `smelt.ref()`. To verify types, flatten the two-model chain into a single CTE query for DuckDB:

```sql
-- DuckDB equivalent of model_A -> model_B chain
WITH model_A AS (
  SELECT CAST(42 AS INTEGER) AS int_col, CAST('hi' AS VARCHAR) AS str_col
)
SELECT UPPER(str_col) AS expr_0, int_col + 1 AS expr_1 FROM model_A
```

This is straightforward since model_A is always a simple `SELECT ... CAST ...` query.

### Generator Changes

Add to `generators.rs`:

1. **`MultiModelScenario` struct** -- Holds the generated model_A columns, model_B expressions, and both SQL strings (smelt-syntax and DuckDB-flattened).

2. **`multi_model_scenario_strategy()`** -- Reuses existing `column_pool_strategy()` for model_A columns and `generate_expr()` for model_B expressions, but:
   - model_A SQL: `SELECT {cast_cols}` (no CTE, no FROM -- this is the full model)
   - model_B SQL: `SELECT {exprs} FROM smelt.ref('model_A')` (uses smelt.ref syntax)
   - DuckDB SQL: `WITH model_A AS (SELECT {cast_cols}) SELECT {exprs} FROM model_A`

3. **Expression generation for model_B** -- The existing `generate_expr()` takes `&[TypedSource]` (column pool). For model_B, the column pool is derived from model_A's output: same column names and types, but no `cast_sql` needed (columns come from the ref). A thin adapter or new function `generate_ref_expr()` can reuse `generate_expr` logic by constructing `TypedSource` entries from model_A's output columns.

## Implementation Steps

### Step 1: Add `MultiModelScenario` and Generator

In `generators.rs`:

```rust
/// A two-model test scenario: model_A provides columns, model_B consumes them via ref.
#[derive(Debug, Clone)]
pub struct MultiModelScenario {
    /// model_A columns (with CAST expressions)
    pub model_a_columns: Vec<TypedSource>,
    /// model_A SQL (smelt syntax): SELECT cast_col AS name, ...
    pub model_a_sql: String,
    /// model_B SQL (smelt syntax): SELECT expr AS alias, ... FROM smelt.ref('model_A')
    pub model_b_sql: String,
    /// Flattened DuckDB SQL: WITH model_A AS (...) SELECT expr AS alias, ... FROM model_A
    pub duckdb_sql: String,
    /// Expected column names in model_B output
    pub model_b_expr_aliases: Vec<String>,
}
```

Add `assemble_multi_model_queries()` function:
- Takes `columns: &[TypedSource]` and `exprs: &[TypedExpr]`
- Returns `MultiModelScenario`
- model_A SQL: `SELECT {cast_cols}` -- plain SELECT with CAST columns, no FROM
- model_B SQL: `SELECT {exprs} FROM smelt.ref('model_A')`
- DuckDB SQL: `WITH model_A AS (SELECT {cast_cols}) SELECT {exprs} FROM model_A`

Add `multi_model_scenario_strategy()`:
- Reuses `column_pool_strategy()` and expression generation
- Returns `impl Strategy<Value = MultiModelScenario>`

### Step 2: Add Salsa Database Setup Helper

In `type_property_tests.rs` (or a new helper module):

```rust
fn setup_multi_model_db(model_a_sql: &str, model_b_sql: &str) -> (Database, PathBuf) {
    let mut db = Database::default();
    let model_a_path = PathBuf::from("models/model_A.sql");
    let model_b_path = PathBuf::from("models/model_B.sql");

    db.set_file_text(model_a_path.clone(), Arc::new(model_a_sql.to_string()));
    db.set_file_text(model_b_path.clone(), Arc::new(model_b_sql.to_string()));
    db.set_all_files(Arc::new(vec![model_a_path.clone(), model_b_path.clone()]));
    db.set_file_project_root(model_a_path, PathBuf::from("."));
    db.set_file_project_root(model_b_path.clone(), PathBuf::from("."));
    db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
    db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));

    (db, model_b_path)
}
```

### Step 3: Add Property Test

In `type_property_tests.rs`:

```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn prop_multi_model_type_inference(scenario in multi_model_scenario_strategy()) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        // Get DuckDB actual types via flattened query
        let actual_types = match duckdb.query_types(&scenario.duckdb_sql) {
            Ok(types) => types,
            Err(_) => return Ok(()),  // Skip invalid SQL
        };

        // Get smelt inferred types via Salsa pipeline
        let (db, model_b_path) = setup_multi_model_db(
            &scenario.model_a_sql,
            &scenario.model_b_sql,
        );
        let typed_schema = db.typed_model_schema(model_b_path);

        // Compare each column
        for (i, actual) in actual_types.iter().enumerate() {
            let smelt_type = typed_schema.columns.get(i)
                .and_then(|c| c.data_type.as_ref())
                .map(|tc| &tc.data_type);

            if let Some(smelt_type) = smelt_type {
                if *smelt_type == DataType::Unknown { continue; }
                match compare_types(smelt_type, &actual.1) {
                    TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
                    TypeMatch::Mismatch => {
                        if find_divergence(smelt_type, &actual.1, "duckdb", &divergences).is_none() {
                            prop_assert!(false,
                                "Multi-model type mismatch col {} ({}):\n  \
                                 smelt: {:?}\n  duckdb: {:?}\n  \
                                 model_A: {}\n  model_B: {}",
                                i, actual.0, smelt_type, actual.1,
                                scenario.model_a_sql, scenario.model_b_sql
                            );
                        }
                    }
                }
            }
        }
    }
}
```

### Step 4: Add Deterministic Smoke Test

Add a `smoke_multi_model_integer_passthrough` test that verifies a simple case: model_A has `CAST(42 AS INTEGER) AS x`, model_B does `SELECT x FROM smelt.ref('model_A')`, and the inferred type is `Integer`.

### Step 5: Expression Filtering for Ref Context

The existing `generate_expr()` produces expressions using column names from the `TypedSource` pool. When model_B references model_A columns, the column names are the same (they come from model_A's SELECT aliases). However, some expression kinds (e.g., `CaseExpr`, `Between`) generate literal values using `cast_sql`, which is not needed in the ref context.

For simplicity, restrict multi-model expressions to kinds that only reference column names:
- `ColumnRef` -- direct passthrough
- `Function` -- function calls on columns
- `Cast` -- CAST of a column
- `BinaryOp` -- arithmetic on columns

This avoids CTE-specific logic in the expression generators. The restriction can be relaxed later.

## Verification

1. Run `cargo test -p smelt-db --test type_property_tests prop_multi_model` to exercise the new property test
2. Run `cargo test -p smelt-db --test type_property_tests smoke_multi_model` for deterministic smoke tests
3. Run the full suite: `cargo test -p smelt-db --test type_property_tests`
4. Increase coverage: `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_multi_model`

## Risks and Mitigations

- **Salsa overhead in proptest**: Creating a fresh `Database::default()` per test case has some overhead. With 128 cases this should be fast enough (<30s). If it becomes slow, consider sharing the database and using `set_file_text` to mutate between cases.

- **Model name resolution**: The `resolve_ref` query resolves `smelt.ref('model_A')` by matching against file stems in `all_files()`. The path must be `models/model_A.sql` for the stem `model_A` to match. This is already the pattern used in existing Salsa tests.

- **Expression compatibility**: Some expressions generated for single-model CTE tests may not work in the ref context (e.g., window functions with partition clauses referencing CTE-specific column patterns). Start with a restricted expression set and expand incrementally.

## Future Extensions

- **Three-model chains**: model_A -> model_B -> model_C, testing transitive type propagation
- **Type narrowing through CAST**: model_B casts a column to a different type, model_C refs model_B and should see the cast type
- **JOIN between two refs**: model_C refs both model_A and model_B, testing multi-source type contexts
- **Wildcard passthrough**: model_B does `SELECT * FROM smelt.ref('model_A')`, testing that all columns and types propagate
