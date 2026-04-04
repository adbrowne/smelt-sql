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

## Phase 4: Temporal Arithmetic Type Inference `[ ]`

**Priority**: Medium — currently returns Unknown, triggers diagnostics on valid SQL.

**Goal**: Add type inference rules for date/time arithmetic in `infer_binary_expr_type`.

**Work**:
- [ ] DATE + INTERVAL → Timestamp
- [ ] TIMESTAMP - TIMESTAMP → Interval
- [ ] INTERVAL + INTERVAL → Interval
- [ ] INTERVAL * numeric → Interval
- [ ] DATE - DATE → Interval
- [ ] Add unit tests for each temporal arithmetic rule
- [ ] Validate against DuckDB for a sample of expressions

**Verification**: `cargo test -p smelt-db` passes, temporal expressions no longer produce Unknown

---

## Phase 5: UNION Type Widening `[ ]`

**Priority**: Medium — `promote_types()` exists but needs systematic coverage.

**Goal**: Implement full promotion matrix in `promote_types()` and create `prop_setop_types.rs`.

**Work**:
- [ ] Implement promotion rules from research doc (SmallInt+Integer→Integer, etc.)
- [ ] Handle Date+Timestamp→Timestamp, Varchar+Text→Text
- [ ] Handle Null promotion (Any+Null→Any nullable)
- [ ] Decimal precision arithmetic for Decimal+Decimal promotion
- [ ] Create `prop_setop_types.rs` testing UNION with varying column types
- [ ] Validate against DuckDB oracle

**Verification**: `cargo test -p smelt-db --test prop_setop_types` passes

---

## Phase 6: CTE Type Proptest Generators `[ ]`

**Priority**: Medium — CTEs tested by unit tests but no combinatorial coverage.

**Goal**: Create `prop_cte_types.rs` generating multi-CTE queries with cross-CTE column references.

**Work**:
- [ ] Generate queries with 1-3 CTEs referencing each other's columns
- [ ] Include type transformations across CTE boundaries
- [ ] Validate type inference of final SELECT against DuckDB
- [ ] Handle recursive CTE type inference (non-recursive term only, per research decision)

**Verification**: `cargo test -p smelt-db --test prop_cte_types` passes

---

## Phase 7: Window Function Proptest Generators `[ ]`

**Priority**: Medium — window functions tested by unit tests, partially by existing proptests.

**Goal**: Create `prop_window_types.rs` with varied window specs.

**Work**:
- [ ] Generate window functions with PARTITION BY, ORDER BY variations
- [ ] Cover ROWS/RANGE/GROUPS frame types
- [ ] Include aggregate functions (SUM, AVG, COUNT) as window functions
- [ ] Include ranking functions (ROW_NUMBER, RANK, DENSE_RANK, NTILE)
- [ ] Validate return types against DuckDB

**Verification**: `cargo test -p smelt-db --test prop_window_types` passes

---

## Phase 8: Parser Depth Limit (Stack Safety) `[ ]`

**Priority**: Medium — prevents stack overflow on adversarial input.

**Goal**: Add depth counter to recursive descent parser, error at depth 256.

**Work**:
- [ ] Add `depth: u32` parameter threaded through recursive parse functions
- [ ] Return error node when depth > 256
- [ ] Add test with deeply nested expression (300+ levels)
- [ ] Verify normal SQL (depth < 50) is unaffected

**Verification**: `cargo test -p smelt-parser` passes, deep nesting test produces error not panic

---

## Phase 9: Parser Error Recovery Tests `[ ]`

**Priority**: Low — only 2 error recovery tests exist, but parser works well in practice.

**Goal**: Add error recovery tests for common incomplete SQL patterns.

**Work**:
- [ ] Missing SELECT list
- [ ] Incomplete CASE (missing END)
- [ ] Incomplete CTE (missing AS/SELECT)
- [ ] Dangling operators (e.g., `SELECT a +`)
- [ ] Missing closing parenthesis
- [ ] Incomplete BETWEEN (missing AND)
- [ ] Verify error recovery produces usable partial AST (not empty)

**Verification**: `cargo test -p smelt-parser` passes with new error recovery tests

---

## Phase 10: Subquery Type Proptest Generators `[ ]`

**Priority**: Low — unit tests exist but no combinatorial coverage.

**Goal**: Create `prop_subquery_types.rs` for scalar subqueries, IN subqueries, EXISTS.

**Work**:
- [ ] Generate scalar subqueries in SELECT and WHERE positions
- [ ] Generate IN (SELECT ...) expressions
- [ ] Generate EXISTS (SELECT ...) expressions
- [ ] Validate types against DuckDB

**Verification**: `cargo test -p smelt-db --test prop_subquery_types` passes

---

## Phase 11: Array Element Type Inference `[ ]`

**Priority**: Low — array subscript returns Unknown currently.

**Goal**: Infer element type for array subscripts and reject mixed-type array literals.

**Work**:
- [ ] Array literal `[1, 2, 3]` → Array(Integer), infer element type
- [ ] Array subscript `arr[i]` → element type of arr
- [ ] Mixed-type array literal → diagnostic error
- [ ] Empty `ARRAY[]` → Array(Unknown) with warning
- [ ] Add unit tests

**Verification**: `cargo test -p smelt-db` passes

---

## Phase 12: Struct Type Inference `[ ]`

**Priority**: Medium — parser already handles ROW/STRUCT syntax but type inference returns Unknown.

**Goal**: Add `Struct` variant to `DataType`, implement type inference for struct literals and field access.

**Context**: The parser already parses `ROW(1, 2, 3)` → `ROW_CONSTRUCTOR` and `STRUCT(1 AS a, 'hello' AS b)` → `STRUCT_LITERAL` syntax nodes. But there's no `Struct` DataType variant and type inference ignores these nodes entirely.

**Work**:
- [ ] Add `Struct(Vec<(String, DataType)>)` variant to `DataType` enum in `crates/smelt-types/src/lib.rs`
- [ ] Handle `ROW_CONSTRUCTOR` in type inference — infer field types positionally (unnamed fields)
- [ ] Handle `STRUCT_LITERAL` in type inference — infer named field types from `expr AS name`
- [ ] Implement struct field access via dot notation (e.g., `s.field`) in type inference
- [ ] Add `Display`/serialization support for the new Struct variant
- [ ] Add unit tests for struct literal type inference
- [ ] Add unit tests for struct field access type inference
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
