# Plan: Parser & Type System Testing Completeness

**Date**: 2026-04-04
**Research**: `docs/research/2026-04-04-parser-type-testing-completeness.md`
**Branch**: `parser-type-testing-completeness`
**PR**: https://github.com/adbrowne/smelt-sql/pull/101

## Design Principle

**Prefer well-defined behaviour and strictness.** smelt should catch errors at compile time, not runtime. When making decisions about type inference, coercion, nullability, or error handling, choose the stricter option — reject ambiguous or unsafe constructs with a clear diagnostic rather than silently accepting them and risking runtime failures. This applies throughout: mixed-type arrays should be rejected, not coerced; nullable precision should be tracked accurately so downstream consumers can rely on it; type mismatches in UNION branches should be flagged, not silently widened.

## Status Key

- `[ ]` — Not started
- `[~]` — In progress
- `[x]` — Complete
- `[!]` — Blocked or needs review

---

## Phase 1: Nested Function Proptest Generators `[x]`

**Priority**: Highest — most common real-world pattern with zero proptest coverage.

**Goal**: Create `prop_nested_functions.rs` in `crates/smelt-db/tests/` that generates nested function calls 2-4 levels deep and validates type inference against DuckDB.

**Work**:
- [x] Add generator strategies for nested function compositions (e.g., `LENGTH(UPPER(col))`, `COALESCE(SUM(x), 0)`, `CAST(EXTRACT(YEAR FROM ts) AS VARCHAR)`)
- [x] Wire into existing DuckDB oracle infrastructure
- [x] Run with 256 cases, register any new divergences
- [x] Verify passes in CI (`cargo test -p smelt-db`)

**Verification**: `cargo test -p smelt-db --test prop_nested_functions` passes

---

## Phase 2: Binary Operator Type Coercion Matrix `[x]`

**Priority**: High — partial coverage exists but no systematic type-pair testing.

**Goal**: Create `prop_coercion_matrix.rs` that tests all type-pair combinations through arithmetic and comparison operators against DuckDB.

**Work**:
- [x] Generate expressions like `CAST(x AS T1) + CAST(y AS T2)` for all type pairs
- [x] Cover arithmetic (+, -, *, /), comparison (<, >, =, !=, <=, >=), and string concat (||)
- [x] Validate inferred types against DuckDB oracle
- [x] Fix any type inference bugs discovered
- [x] Register legitimate divergences in `divergences.rs`

**Verification**: `cargo test -p smelt-db --test prop_coercion_matrix` passes

---

## Phase 3: COALESCE/CASE Nullability Precision `[x]`

**Priority**: Medium — doesn't cause runtime errors but weakens type guarantees.

**Goal**: Fix over-nullability in `type_inference.rs` for COALESCE, CASE with ELSE, and CAST.

**Work**:
- [x] COALESCE: non-nullable when at least one arg is non-nullable or a non-null literal
- [x] CASE WHEN ... ELSE: non-nullable when ELSE present AND all branches non-nullable
- [x] CAST of non-nullable expr to non-nullable type: preserve non-nullable
- [x] IFNULL/NVL: same as COALESCE with 2 args
- [x] Add unit tests for each case in type inference tests
- [x] Verify existing tests still pass

**Verification**: `cargo test -p smelt-db` passes, new nullability unit tests pass

---

## Phase 4: Temporal Arithmetic Type Inference `[x]`

**Priority**: Medium — currently returns Unknown, triggers diagnostics on valid SQL.

**Goal**: Add type inference rules for date/time arithmetic in `infer_binary_expr_type`.

**Work**:
- [x] DATE + INTERVAL → Timestamp
- [x] TIMESTAMP - TIMESTAMP → Interval
- [x] INTERVAL + INTERVAL → Interval
- [x] INTERVAL * numeric → Interval
- [x] DATE - DATE → Interval
- [x] Add unit tests for each temporal arithmetic rule
- [x] Validate against DuckDB for a sample of expressions

**Verification**: `cargo test -p smelt-db` passes, temporal expressions no longer produce Unknown

---

## Phase 5: UNION Type Widening `[x]`

**Priority**: Medium — `promote_types()` exists but needs systematic coverage.

**Goal**: Implement full promotion matrix in `promote_types()` and create `prop_setop_types.rs`.

**Work**:
- [x] Implement promotion rules from research doc (SmallInt+Integer→Integer, etc.)
- [x] Handle Date+Timestamp→Timestamp, Varchar+Text→Text
- [x] Handle Null promotion (Any+Null→Any nullable)
- [x] Decimal precision arithmetic for Decimal+Decimal promotion
- [x] Create `prop_setop_types.rs` testing UNION with varying column types
- [x] Validate against DuckDB oracle

**Verification**: `cargo test -p smelt-db --test prop_setop_types` passes

---

## Phase 6: CTE Type Proptest Generators `[x]`

**Priority**: Medium — CTEs tested by unit tests but no combinatorial coverage.

**Goal**: Create `prop_cte_types.rs` generating multi-CTE queries with cross-CTE column references.

**Work**:
- [x] Generate queries with 1-3 CTEs referencing each other's columns
- [x] Include type transformations across CTE boundaries
- [x] Validate type inference of final SELECT against DuckDB
- [x] Handle recursive CTE type inference (non-recursive term only, per research decision)

**Verification**: `cargo test -p smelt-db --test prop_cte_types` passes

---

## Phase 7: Window Function Proptest Generators `[x]`

**Priority**: Medium — window functions tested by unit tests, partially by existing proptests.

**Goal**: Create `prop_window_types.rs` with varied window specs.

**Work**:
- [x] Generate window functions with PARTITION BY, ORDER BY variations
- [x] Cover ROWS/RANGE/GROUPS frame types
- [x] Include aggregate functions (SUM, AVG, COUNT) as window functions
- [x] Include ranking functions (ROW_NUMBER, RANK, DENSE_RANK, NTILE)
- [x] Validate return types against DuckDB

**Verification**: `cargo test -p smelt-db --test prop_window_types` passes

---

## Phase 8: Parser Depth Limit (Stack Safety) `[x]`

**Priority**: Medium — prevents stack overflow on adversarial input.

**Goal**: Add depth counter to recursive descent parser, error at depth 256.

**Work**:
- [x] Add `depth: u32` parameter threaded through recursive parse functions
- [x] Return error node when depth > 256
- [x] Add test with deeply nested expression (300+ levels)
- [x] Verify normal SQL (depth < 50) is unaffected

**Verification**: `cargo test -p smelt-parser` passes, deep nesting test produces error not panic

---

## Phase 9: Parser Error Recovery Tests `[x]`

**Priority**: Low — only 2 error recovery tests exist, but parser works well in practice.

**Goal**: Add error recovery tests for common incomplete SQL patterns.

**Work**:
- [x] Missing SELECT list
- [x] Incomplete CASE (missing END)
- [x] Incomplete CTE (missing AS/SELECT)
- [x] Dangling operators (e.g., `SELECT a +`)
- [x] Missing closing parenthesis
- [x] Incomplete BETWEEN (missing AND)
- [x] Verify error recovery produces usable partial AST (not empty)

**Verification**: `cargo test -p smelt-parser` passes with new error recovery tests

---

## Phase 10: Subquery Type Proptest Generators `[x]`

**Priority**: Low — unit tests exist but no combinatorial coverage.

**Goal**: Create `prop_subquery_types.rs` for scalar subqueries, IN subqueries, EXISTS.

**Work**:
- [x] Generate scalar subqueries in SELECT and WHERE positions
- [x] Generate IN (SELECT ...) expressions
- [x] Generate EXISTS (SELECT ...) expressions
- [x] Validate types against DuckDB

**Verification**: `cargo test -p smelt-db --test prop_subquery_types` passes

---

## Phase 11: Array Element Type Inference `[x]`

**Priority**: Low — array subscript returns Unknown currently.

**Goal**: Infer element type for array subscripts and reject mixed-type array literals.

**Work**:
- [x] Array literal `[1, 2, 3]` → Array(Integer), infer element type
- [x] Array subscript `arr[i]` → element type of arr
- [x] Mixed-type array literal → diagnostic error
- [x] Empty `ARRAY[]` → Array(Unknown) with warning
- [x] Add unit tests

**Verification**: `cargo test -p smelt-db` passes

---

## Phase 12: Struct Type Inference `[x]`

**Priority**: Medium — parser already handles ROW/STRUCT syntax but type inference returns Unknown.

**Goal**: Add `Struct` variant to `DataType`, implement type inference for struct literals and field access.

**Context**: The parser already parses `ROW(1, 2, 3)` → `ROW_CONSTRUCTOR` and `STRUCT(1 AS a, 'hello' AS b)` → `STRUCT_LITERAL` syntax nodes. But there's no `Struct` DataType variant and type inference ignores these nodes entirely.

**Work**:
- [x] Add `Struct(Vec<(String, DataType)>)` variant to `DataType` enum in `crates/smelt-types/src/lib.rs`
- [x] Handle `ROW_CONSTRUCTOR` in type inference — infer field types positionally (unnamed fields)
- [x] Handle `STRUCT_LITERAL` in type inference — infer named field types from `expr AS name`
- [x] Implement struct field access via dot notation (e.g., `s.field`) in type inference
- [x] Add `Display`/serialization support for the new Struct variant
- [x] Add unit tests for struct literal type inference
- [x] Add unit tests for struct field access type inference
- [ ] Validate struct type inference against DuckDB for sample expressions
- [ ] Update proptest generators to include struct expressions (optional — can be a follow-up)

**Verification**: `cargo test -p smelt-db` passes, struct literals and field access infer correct types

---

## Phase 13: Type Semantics Documentation `[ ]`

**Priority**: Low — documentation only, no code changes.

**Goal**: Create `docs/type_semantics.md` documenting where smelt differs from backends.

**Work**:
- [ ] Integer division is truncating
- [ ] SUM of integers returns BigInt (not Decimal)
- [ ] String functions return Text (not Varchar)
- [ ] CEIL/FLOOR of Double returns Double
- [ ] UNION type promotion rules
- [ ] Temporal arithmetic rules
- [ ] Array type rules
- [ ] Struct type rules and field access semantics

**Verification**: File exists and is accurate

---

## Decisions Log

Decisions made during implementation that may need review. These are filled in as work progresses.

1. **Float promotion priority (Phase 2)**: Added Float to the numeric promotion chain between Double and Decimal: `Double > Float > Decimal > BigInt > Integer > SmallInt`. This matches DuckDB's behavior. Previously Float was missing entirely, causing `Float + Integer` to incorrectly return Integer.

2. **Division type divergences (Phase 2)**: DuckDB v1.5+ uses non-truncating division (all division returns Double). smelt intentionally uses truncating integer division (Integer/Integer → Integer) and preserves Decimal type for Decimal division. Registered 5 new divergences: `integer_division`, `smallint_division`, `bigint_division`, `decimal_division`, `float_division`. All marked as `ByDesign`.

3. **% (modulo) not tested (Phase 2)**: The parser doesn't support `%` as a binary operator, so it was excluded from the coercion matrix. The plan item mentioned `%` but it's not applicable.

4. **promote_types() family-gated promotion (Phase 5)**: Restructured `promote_types()` to gate numeric, string, and temporal promotion by type family (`is_numeric()`, `is_string()`, `is_temporal()`). Previously, wildcard arms like `(DataType::Float, _)` would incorrectly match cross-family pairs (e.g., Float+Varchar → Float). Now cross-family combinations correctly return Unknown. DuckDB is more permissive (e.g., Boolean+Integer → Integer, Numeric+Varchar → Varchar), but per the design principle we prefer strictness.

5. **INTERSECT/EXCEPT type inference (Phase 5)**: Added `has_set_operation()` and `set_operation_select()` AST methods that handle all three set operations (UNION, INTERSECT, EXCEPT). Updated `infer_select_column_types()` to use these generic methods instead of the UNION-specific ones. All set operations use the same type promotion logic.

6. **`build_subquery_context()` made public (Phase 6)**: Made `build_subquery_context()` public so that proptest files can properly resolve CTE columns when parsing full SQL with WITH clauses. Without this, tests using `TypeContext::new()` + `infer_select_column_types()` would not see CTE-defined columns (they'd all be Unknown). This is consistent with the "pure function" architecture — tests call the same context-building function that the production code uses.

---

## Session Log

Each Claude Code session records what it accomplished here.

### Session 1 — 2026-04-04

**Phase**: 1 (Nested Function Proptest Generators)
**Status**: Complete

**What was done**:
- `prop_nested_functions.rs` was already written with full infrastructure: WrapperFunc-based nesting chains (2-4 levels deep), 256-case proptest, and 8 deterministic smoke tests covering string→string, string→numeric, numeric→numeric, and COALESCE nesting patterns.
- Cleaned up dead code warnings: removed unused `WrapperInput` enum and `accepts` field from `WrapperFunc`, removed empty `numeric_to_string_wrappers()` placeholder.
- All 35 tests pass (256 proptest cases + helper module tests + smoke tests). Zero warnings from clippy.

**Decisions**: None — implementation was already in good shape, only cleanup needed.

### Session 2 — 2026-04-04

**Phase**: 2 (Binary Operator Type Coercion Matrix)
**Status**: Complete

**What was done**:
- Created `prop_coercion_matrix.rs` with 42 tests total:
  - 3 proptests (256 cases each): arithmetic coercion, same-type comparison, cross-numeric comparison
  - 3 exhaustive deterministic tests: full 6×6×4=144 arithmetic matrix, 10×6=60 same-type comparisons, string concat
  - 10 smoke tests for specific coercion rules
- Fixed Float type promotion bug in `type_inference.rs`: Float was missing from the numeric promotion chain. Extracted duplicated promotion logic into `promote_numeric_operands()` helper.
- Updated `promote_numeric_type()` in `generators.rs` to include Float.
- Registered 5 new divergences in `divergences.rs` for DuckDB's non-truncating division behavior.

**Decisions**: See Decisions Log entries 1-3.

### Session 3 — 2026-04-04

**Phase**: 3 (COALESCE/CASE Nullability Precision)
**Status**: Complete

**What was done**:
- Fixed COALESCE nullability: now returns non-nullable when at least one argument is non-nullable or a non-null literal. Previously always returned `nullable: true`.
- Fixed CASE WHEN nullability: now returns non-nullable when an ELSE clause is present AND all branches (THEN + ELSE) are non-nullable. Without ELSE, remains nullable (implicit NULL default).
- Fixed CAST nullability: now preserves the input expression's nullability. Previously always returned `nullable: true`.
- Added `Ifnull` variant to `SqlFunction` enum with `NVL` as a dialect alias. IFNULL uses the same nullability logic as COALESCE (non-nullable when either arg is non-nullable).
- Added 4 new unit tests (16 assertions total): `test_coalesce_nullability`, `test_case_nullability`, `test_cast_nullability`, `test_ifnull_nullability`.
- Added `infer_sql` and `infer_sql_with_ctx` test helpers for parsing SQL and running type inference in tests.
- All 101 lib tests pass, zero clippy warnings.

**Decisions**: None — straightforward implementation of well-defined nullability rules.

### Session 4 — 2026-04-04

**Phase**: 4 (Temporal Arithmetic Type Inference)
**Status**: Complete

**What was done**:
- Added temporal arithmetic type inference rules to `infer_binary_expr_type` in `type_inference.rs`:
  - `+` operator: DATE/TIMESTAMP/TIME + INTERVAL → appropriate temporal type, INTERVAL + INTERVAL → Interval
  - `-` operator: DATE-DATE, TIMESTAMP-TIMESTAMP, TIME-TIME → Interval; temporal - INTERVAL → same temporal type; INTERVAL - INTERVAL → Interval
  - `*`/`/` operators: INTERVAL * numeric → Interval, INTERVAL / numeric → Interval
- Split the previous combined `"+" | "*" | "/"` arm into separate `"+"` and `"*" | "/"` arms to handle temporal types before falling through to numeric promotion.
- Added 5 new unit tests (16 assertions total): `test_temporal_arithmetic_date_interval`, `test_temporal_arithmetic_timestamp_interval`, `test_temporal_arithmetic_interval_ops`, `test_temporal_arithmetic_time`, `test_temporal_arithmetic_with_columns`.
- All 106 lib tests pass, zero clippy warnings.

**Decisions**: None — temporal arithmetic rules follow standard SQL semantics and match DuckDB behavior.

### Session 5 — 2026-04-04

**Phase**: 5 (UNION Type Widening)
**Status**: Complete

**What was done**:
- Restructured `promote_types()` to gate promotion by type family:
  - Numeric promotion now only applies when both types are numeric (via `is_numeric()`)
  - String promotion only when both are strings (via `is_string()`)
  - Temporal promotion only when both are temporal (via `is_temporal()`)
  - Cross-family combinations return Unknown (strict by design)
- Added Float to `promote_types()` numeric hierarchy: `Double > Float > Decimal > BigInt > Integer > SmallInt`
- Added Null type handling: `Null + T → T (nullable)`
- Added `has_set_operation()` and `set_operation_select()` AST methods handling UNION/INTERSECT/EXCEPT generically
- Updated `infer_select_column_types()` to use new generic set operation methods
- Created `prop_setop_types.rs` with 42 tests:
  - 3 proptests (256 cases each): numeric set op promotion, 3-way numeric union, multi-column union
  - 3 exhaustive deterministic tests: 6×6 numeric matrix, 10×6 same-type all set ops, 10×10 all-type cross matrix
  - 10 smoke tests for specific promotion rules
- Added 7 new unit tests in `type_inference.rs`: promote_types numeric hierarchy, null handling, temporal, string, decimal precision, union inference, intersect/except inference
- All 113 lib tests + 42 proptest tests pass, zero clippy warnings.

**Decisions**: See Decisions Log entries 4-5.

### Session 6 — 2026-04-04

**Phase**: 6 (CTE Type Proptest Generators)
**Status**: Complete

**What was done**:
- Created `prop_cte_types.rs` with 43 tests total:
  - 4 proptests (256 cases each): single-CTE passthrough, two-CTE with CAST transforms, two-CTE with type-specific transforms (ArithmeticIdentity, StringFunc), three-CTE chain with cascading transforms
  - 3 exhaustive deterministic tests: all 10 base types through single CTE, 10×10 two-CTE CAST matrix, 10×10 branching CTE (cte3 references both cte1 and cte2 via CROSS JOIN)
  - 10 smoke tests: integer passthrough, multi-column, type widening chain, 3-CTE chain, aggregation in CTE, function transform, COALESCE, branching CROSS JOIN, CASE expression, recursive CTE with column list
- Made `build_subquery_context()` public in `type_inference.rs` so tests can properly resolve CTE columns from WITH clauses. Previously tests using `TypeContext::new()` couldn't see CTE-defined columns, causing all CTE column references to infer as Unknown.
- TypeTransform enum covers: Identity, Cast(target), ArithmeticIdentity (numeric only), StringFunc (varchar only), Coalesce, CaseExpr
- All 43 tests pass, zero clippy warnings.

**Decisions**: See Decisions Log entry 6.

### Session 7 — 2026-04-04

**Phase**: 7 (Window Function Proptest Generators)
**Status**: Complete

**What was done**:
- `prop_window_types.rs` was already written with comprehensive coverage (46 tests total):
  - 4 proptests (256 cases each): general window func types, ranking functions, value functions (LAG/LEAD/etc.), aggregate-as-window functions
  - 3 exhaustive deterministic tests: all 16 window funcs × 6 types, all frame specs × aggregate windows, all OVER clause variations
  - 13 smoke tests: ROW_NUMBER/RANK/DENSE_RANK/NTILE→BigInt, CUME_DIST/PERCENT_RANK→Double, LAG preserves VARCHAR, LEAD preserves TIMESTAMP, FIRST_VALUE preserves DATE, AVG→Double, COUNT→BigInt, SUM with frame spec, full PARTITION BY+ORDER BY+frame
- Fixed compilation error: `DataType::Varchar { length: None }` → `DataType::Varchar { max_length: None }` (field was renamed)
- Removed unused `to_smelt_type()` method to eliminate dead_code warning
- All 46 tests pass, zero clippy warnings.

**Decisions**: None — implementation was already comprehensive, only minor fixes needed.

### Session 8 — 2026-04-04

**Phase**: 8 (Parser Depth Limit — Stack Safety)
**Status**: Complete

**What was done**:
- Added `MAX_PARSE_DEPTH` constant (256) and `depth: u32` field to the `Parser` struct.
- Added `too_deep()` helper method that checks `depth >= MAX_PARSE_DEPTH` and emits an error diagnostic.
- Guarded two key recursive entry points with depth tracking:
  - `parse_expression()` — covers all expression recursion (parenthesized exprs, function args, CASE branches, array literals, etc.)
  - `parse_select_stmt()` — covers statement recursion (subqueries, CTEs, set operations)
- Added 4 new tests:
  - `test_deeply_nested_parens_produces_error` — 300 nested parens, verifies error not panic
  - `test_deeply_nested_subqueries_produces_error` — 300 nested subqueries, verifies error not panic
  - `test_normal_nesting_depth_unaffected` — COALESCE nesting ~5 levels, no errors
  - `test_moderate_nesting_depth_unaffected` — 40 nested parens, no errors
- All 223 parser tests pass, zero clippy warnings.

**Decisions**: Depth tracking at `parse_expression` and `parse_select_stmt` is sufficient because these are the two recursive entry points through which all nesting passes. No need to track `parse_unary_expr` separately since it always goes through `parse_expression` first.

### Session 9 — 2026-04-04

**Phase**: 9 (Parser Error Recovery Tests)
**Status**: Complete

**What was done**:
- Added 16 new error recovery tests to `parser.rs` covering all Phase 9 work items:
  - Missing SELECT list: `test_error_recovery_missing_select_list`, `test_error_recovery_select_only`
  - Incomplete CASE: `test_error_recovery_incomplete_case_missing_end`, `test_error_recovery_incomplete_case_missing_then`
  - Incomplete CTE: `test_error_recovery_incomplete_cte_missing_as`, `test_error_recovery_incomplete_cte_missing_select`
  - Dangling operators: `test_error_recovery_dangling_operator_plus`, `test_error_recovery_dangling_operator_equals`
  - Missing closing paren: `test_error_recovery_missing_closing_paren`, `test_error_recovery_missing_closing_paren_in_function`
  - Incomplete BETWEEN: `test_error_recovery_incomplete_between_missing_and`, `test_error_recovery_between_missing_upper_bound`
  - Partial AST verification: `test_error_recovery_partial_ast_has_content`, `test_error_recovery_completely_invalid_input`, `test_error_recovery_empty_input`, `test_error_recovery_multiple_errors_still_produces_tree`
- Added `parse_with_errors()` test helper that asserts errors exist and verifies the root FILE node is present.
- All 239 parser tests pass (16 new + 223 existing), zero clippy warnings.

**Decisions**: None — the existing parser error recovery mechanisms (sync_to, expect, error nodes) already handle all the tested cases well. No parser changes were needed, only test additions.

### Session 10 — 2026-04-04

**Phase**: 10 (Subquery Type Proptest Generators)
**Status**: Complete

**What was done**:
- Created `prop_subquery_types.rs` with 47 tests total:
  - 4 proptests (256 cases each): scalar subquery in SELECT, scalar subquery from CTE, two scalar subqueries with different types, IN subquery type
  - 7 exhaustive deterministic tests: all 10 base types through scalar subquery, all 10 types through EXISTS, 10×10 two-scalar-subquery pairs, 10×10 IN subquery filter with select type preservation, all 10 types through nested scalar subqueries, all 10 types through scalar+EXISTS combined, 6 numeric types through scalar subquery arithmetic
  - 10 smoke tests: integer/varchar/boolean/timestamp scalar subqueries, CTE-based scalar subquery, nested scalar subquery, IN subquery → Boolean, EXISTS → Boolean, scalar subquery with arithmetic, scalar+EXISTS combined
- SQL generation helpers cover: scalar subqueries in SELECT position, scalar subqueries from CTEs, nested scalar subqueries (2 levels deep), IN subqueries, EXISTS subqueries, scalar subquery with arithmetic, combined scalar+EXISTS, SELECT with IN subquery filter
- All 47 tests pass, zero clippy warnings.

**Decisions**: None — subquery type inference was already correctly implemented (scalar → first column type always nullable, IN → Boolean nullable, EXISTS → Boolean non-nullable). No code changes needed, only test additions.

### Session 11 — 2026-04-04

**Phase**: 11 (Array Element Type Inference)
**Status**: Complete

**What was done**:
- Added `ArrayLiteral` AST wrapper struct with `elements()` method to `ast.rs`
- Added `as_array_literal()`, `as_array_subscript()`, `as_array_slice()` methods to `Expr` in `ast.rs`
- Implemented `infer_array_literal_type()` in `type_inference.rs`:
  - Infers element type from all elements, uses `promote_types()` for compatible but different types
  - Returns `None` (→ Unknown) for mixed-type arrays that can't be promoted (e.g., Integer + Text)
  - Returns `Array(Unknown)` for empty `ARRAY[]`
  - Handles NULL elements (compatible with any element type)
  - Array literal itself is non-nullable
- Implemented `infer_array_subscript_type()`: extracts element type from Array base, always nullable (out-of-bounds possible)
- Implemented `infer_array_slice_type()`: returns same Array type as base, nullable
- Added 8 unit tests: integer/string/empty/with-null/numeric-promotion/mixed-types-rejected/subscript/slice
- All 121 lib tests pass, zero clippy warnings.

**Decisions**: None — straightforward implementation. `DataType::Array(Box<DataType>)` variant already existed in smelt-types.

### Session 12 — 2026-04-04

**Phase**: 12 (Struct Type Inference)
**Status**: Complete

**What was done**:
- Added `Struct(Vec<(String, DataType)>)` variant to `DataType` enum in `smelt-types/src/lib.rs` with `to_sql()` Display support (e.g., `STRUCT(a INTEGER, b TEXT)`)
- Added `RowConstructor` and `StructLiteral` AST wrapper structs in `smelt-parser/src/ast.rs`:
  - `RowConstructor::elements()` returns child expressions
  - `StructLiteral::fields()` returns `(Expr, Option<String>)` pairs, parsing the `expr AS name` pattern from the CST
- Added `as_row_constructor()` and `as_struct_literal()` accessor methods to `Expr`
- Implemented `infer_row_constructor_type()`: ROW(1, 'hello', TRUE) → Struct with positional field names (v1, v2, v3)
- Implemented `infer_struct_literal_type()`: STRUCT(1 AS a, 'hello' AS b) → Struct with named fields; unnamed fields get positional names
- Implemented struct field access via qualified column reference fallback: when `qualifier.name` doesn't resolve as a column, checks if `qualifier` resolves to a Struct-typed column and looks up `name` as a field (case-insensitive)
- Added 8 new unit tests: row_constructor_type, struct_literal_named_fields, struct_literal_unnamed_fields, struct_literal_mixed_named_unnamed, struct_field_access, struct_field_access_case_insensitive, struct_display
- All 128 lib tests pass, zero clippy warnings
- Deferred: DuckDB validation (not available in worktree) and proptest generators (marked optional in plan)

**Decisions**: Struct field access is implemented as a fallback in the column reference resolution path rather than a separate AST pattern. When `s.name` is parsed as a qualified column ref with qualifier `s` and name `name`, and no column `s.name` exists, we check if bare column `s` has Struct type and resolve `name` as a field. This avoids parser changes and naturally handles the SQL ambiguity between `table.column` and `struct.field`.
