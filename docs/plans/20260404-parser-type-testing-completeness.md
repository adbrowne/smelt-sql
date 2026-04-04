# Plan: Parser & Type System Testing Completeness

**Date**: 2026-04-04
**Research**: `docs/research/2026-04-04-parser-type-testing-completeness.md`
**Branch**: `parser-type-testing-completeness`
**PR**: https://github.com/adbrowne/smelt-sql/pull/101

## Status Key

- `[ ]` — Not started
- `[~]` — In progress
- `[x]` — Complete
- `[!]` — Blocked or needs review

---

## Phase 1: Nested Function Proptest Generators `[ ]`

**Priority**: Highest — most common real-world pattern with zero proptest coverage.

**Goal**: Create `prop_nested_functions.rs` in `crates/smelt-db/tests/` that generates nested function calls 2-4 levels deep and validates type inference against DuckDB.

**Work**:
- [ ] Add generator strategies for nested function compositions (e.g., `LENGTH(UPPER(col))`, `COALESCE(SUM(x), 0)`, `CAST(EXTRACT(YEAR FROM ts) AS VARCHAR)`)
- [ ] Wire into existing DuckDB oracle infrastructure
- [ ] Run with 256 cases, register any new divergences
- [ ] Verify passes in CI (`cargo test -p smelt-db`)

**Verification**: `cargo test -p smelt-db --test prop_nested_functions` passes

---

## Phase 2: Binary Operator Type Coercion Matrix `[ ]`

**Priority**: High — partial coverage exists but no systematic type-pair testing.

**Goal**: Create `prop_coercion_matrix.rs` that tests all type-pair combinations through arithmetic and comparison operators against DuckDB.

**Work**:
- [ ] Generate expressions like `CAST(x AS T1) + CAST(y AS T2)` for all type pairs
- [ ] Cover arithmetic (+, -, *, /, %), comparison (<, >, =), and string concat (||)
- [ ] Validate inferred types against DuckDB oracle
- [ ] Fix any type inference bugs discovered
- [ ] Register legitimate divergences in `divergences.rs`

**Verification**: `cargo test -p smelt-db --test prop_coercion_matrix` passes

---

## Phase 3: COALESCE/CASE Nullability Precision `[ ]`

**Priority**: Medium — doesn't cause runtime errors but weakens type guarantees.

**Goal**: Fix over-nullability in `type_inference.rs` for COALESCE, CASE with ELSE, and CAST.

**Work**:
- [ ] COALESCE: non-nullable when at least one arg is non-nullable or a non-null literal
- [ ] CASE WHEN ... ELSE: non-nullable when ELSE present AND all branches non-nullable
- [ ] CAST of non-nullable expr to non-nullable type: preserve non-nullable
- [ ] IFNULL/NVL: same as COALESCE with 2 args
- [ ] Add unit tests for each case in type inference tests
- [ ] Verify existing tests still pass

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

*(Empty — will be populated during implementation)*

---

## Session Log

Each Claude Code session records what it accomplished here.

*(Empty — will be populated during loop execution)*
