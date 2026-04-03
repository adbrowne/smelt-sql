# Research: Parser Testing

**Date**: 2026-04-03
**Topic**: How parser testing is structured across the smelt codebase
**Branch**: main
**Commit**: 83afae2

## Summary

The smelt parser uses a multi-tier testing strategy: 166+ unit tests for specific SQL features, property-based round-trip tests (proptest) to verify parse-print idempotency, and fuzz testing to guarantee no panics on arbitrary input. The smelt-db crate adds a second layer of property-based tests that verify type inference against real DuckDB/Spark backends, exercising the parser indirectly with generated SQL.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-parser/src/parser.rs` | 156 unit tests for SQL features | L2258-3758 |
| `crates/smelt-parser/src/lexer.rs` | 5 lexer tokenization tests | L419-476 |
| `crates/smelt-parser/src/printer.rs` | 40 round-trip print tests | L599-840 |
| `crates/smelt-parser/src/lib.rs` | 5 frontmatter stripping tests | L63-112 |
| `crates/smelt-parser/tests/proptest_generators.rs` | Grammar-based SQL generators for proptest | L1-316 |
| `crates/smelt-parser/tests/proptest_round_trip.rs` | 8 property tests + 12 edge cases | L1-192 |
| `crates/smelt-parser/fuzz/fuzz_targets/parse_never_panics.rs` | Fuzz: parser never panics | L1-17 |
| `crates/smelt-parser/fuzz/fuzz_targets/round_trip.rs` | Fuzz: round-trip stability | L1-42 |
| `crates/smelt-db/tests/type_property_tests.rs` | Type inference vs DuckDB/Spark | L1-671 |
| `crates/smelt-db/tests/type_conformance_tests.rs` | Cast-wrapped type conformance | L1-303 |
| `crates/smelt-db/tests/prop_helpers/generators.rs` | Type-aware SQL expression generators | full file |
| `crates/smelt-db/tests/prop_helpers/divergences.rs` | Known backend type divergences | L1-235 |
| `crates/smelt-db/tests/prop_helpers/type_comparison.rs` | Exact/Compatible/Mismatch comparison | L1-136 |
| `crates/smelt-db/tests/prop_helpers/duckdb_oracle.rs` | DuckDB execution oracle | L1-92 |
| `crates/smelt-db/tests/prop_helpers/spark_oracle.rs` | Spark Docker oracle | full file |
| `crates/smelt-db/src/type_inference.rs` | Unit tests for type inference logic | L1535-1854 |
| `crates/smelt-db/src/schema.rs` | Schema structure unit tests | L204-272 |

## Architecture & Data Flow

### Tier 1: Parser Unit Tests (smelt-parser)

Each test follows the same pattern:
```
SQL string → parse(sql) → assert errors empty → inspect CST/AST
```

Tests verify two things:
1. **No parse errors**: `assert!(parse.errors.is_empty())`
2. **Correct structure**: Navigate CST via `parse.syntax()` and typed AST wrappers (`File::cast()`, `SelectStmt::cast()`, `.refs()`, `.sources()`)

Error recovery tests intentionally produce errors and assert on error messages:
```rust
// parser.rs:2317-2330
let parse = parse("SELECT * FROM users JOIN");
assert!(!parse.errors.is_empty());
assert!(parse.errors[0].message.contains("table"));
```

### Tier 2: Property-Based Tests (smelt-parser)

Grammar-based generators in `proptest_generators.rs` compose valid SQL by construction:
- `arb_identifier()` → `arb_column_ref()` → `arb_select_list()` → `arb_simple_select()`
- `arb_join_clause()`, `arb_where_clause()`, `arb_group_by_clause()` compose into `arb_any_select()`
- Non-recursive expression generation avoids stack overflow

Round-trip property (`proptest_round_trip.rs:22-57`):
```
parse(sql) → print(cst) → parse(printed) → assert no errors + AST matches
```

Config: 100 cases per property by default, 1000 for panic tests. Overridable via `PROPTEST_CASES`.

### Tier 3: Fuzz Testing (smelt-parser)

Two targets under `crates/smelt-parser/fuzz/`:
- **parse_never_panics**: Arbitrary bytes → `parse()` must not panic (110K+ executions, zero crashes)
- **round_trip**: If parse succeeds without errors, print→reparse must also succeed

### Tier 4: Type Inference Property Tests (smelt-db)

A separate generator suite in `prop_helpers/generators.rs` produces **typed** SQL expressions:
- `TypedSource`: CTE column with known type and cast SQL
- `TypedExpr`: Expression with expected smelt DataType
- `QueryShape`: Scalar, GroupBy, GroupByHaving, Window, Distinct
- `ExprKind`: ColumnRef, Cast, Function, BinaryOp, CaseExpr, Between, WindowFunc, etc.

The flow:
```
generate typed CTE+expression → parse with smelt → infer types
                               → execute on DuckDB/Spark → get actual types
                               → compare (Exact/Compatible/Mismatch)
```

Multi-model tests (`prop_multi_model_type_inference`, `prop_three_model_type_inference`, `prop_join_type_inference`) set up Salsa databases with multiple models connected via `smelt.ref()` to test cross-model type propagation.

## Current Behavior

### What the tests cover

- **SQL syntax**: SELECT, FROM, WHERE, JOIN (all types), GROUP BY, HAVING, ORDER BY, LIMIT/OFFSET, CTEs (WITH/RECURSIVE), UNION, DISTINCT/DISTINCT ON, QUALIFY
- **Expressions**: CASE, CAST, BETWEEN, IN, EXISTS, subqueries, unary operators, window functions (OVER, PARTITION BY, frame specs)
- **smelt extensions**: `smelt.ref()` with named params (`=>` syntax), `smelt.metric()`
- **Operators**: JSON (`->`, `->>`), regex (`~`, `~*`), array (ANY/ALL), concat (`||`)
- **Error recovery**: Missing tokens, incomplete clauses
- **Printer**: Keyword uppercasing, whitespace normalization, round-trip stability
- **Type inference**: Literals, functions (70+ functions), aggregates, operators, CTEs, cross-model refs

### What the tests do NOT cover (or cover lightly)

- **Diagnostic messages** in smelt-db: `file_diagnostics()` is tested indirectly but no dedicated test suite for diagnostic text/positions
- **LSP integration**: No parser tests from the LSP layer
- **Incremental reparsing**: No tests verifying Salsa cache invalidation behavior
- **Large/complex queries**: Proptest generators max out at moderate complexity

## Related Patterns

### Divergence Registry (prop_helpers/divergences.rs)

Known type differences between smelt and backends are registered rather than failing tests:
- `DivergenceStatus::KnownBug` — tracked for future fix
- `DivergenceStatus::ByDesign` — intentional difference
- `DivergenceStatus::BackendSpecific` — backend-specific behavior

Examples: SUM(INTEGER) returns Decimal(38,0) in smelt but HUGEINT in DuckDB; string functions return Varchar in DuckDB but Text in smelt.

### Type Comparison (prop_helpers/type_comparison.rs)

Three-level matching: Exact (identical), Compatible (semantically equivalent like Text/Varchar), Mismatch. This avoids false failures from inconsequential type differences.

### Oracle Pattern (prop_helpers/duckdb_oracle.rs, spark_oracle.rs)

`TypeOracle` trait abstracts backend-specific type querying. DuckDB uses in-memory connection; Spark uses a persistent Docker container with sentinel-based output parsing. Invalid SQL returns `Err` and the test case is skipped.

## Test Coverage

| Layer | Test Count | Cases Generated | Focus |
|-------|-----------|----------------|-------|
| Lexer unit | 5 | 5 | Token streams |
| Parser unit | 156 | 156 | SQL feature coverage + error recovery |
| Printer unit | 40 | 40 | Round-trip formatting |
| Lib unit | 5 | 5 | Frontmatter stripping |
| Parser proptest | 8 properties + 12 edge | ~800 + 2000 (panic) | Round-trip + no-panic |
| Parser fuzz | 2 targets | 110K+ | Arbitrary input safety |
| Type inference unit | ~30 | 30 | Literal/function/CTE inference |
| Type property | 4 properties | 256-1024 per prop | Type correctness vs backends |
| Type conformance | 1 property | 256 | Cast-wrapped exact match |
| Schema unit | ~5 | 5 | Column lookup |

## Open Questions

1. **No snapshot testing**: The codebase uses direct CST/AST inspection instead of snapshot files (e.g., insta). Was this a deliberate choice, or would snapshot testing be welcome for regression catching?
2. **Printer test coverage**: The printer has 40 tests but the proptest round-trip is the primary validation — are there known printer formatting gaps?
3. **Diagnostic testing**: smelt-db diagnostics (undefined refs, type errors) are tested indirectly through `file_diagnostics()` in `lib.rs` but there's no dedicated diagnostic test suite — is this a gap to address?
