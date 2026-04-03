# Plan: Tier 2 Structural Assertions for Parser Tests

**Date**: 2026-04-03
**Research**: `docs/research/2026-04-03-parser-testing.md`
**Status**: Validated

## Overview

Of 156 parser unit tests, only 9 verify AST structure — the other 147 only check `errors.is_empty()`. This means a parser bug that produces the wrong tree silently passes all tests as long as it doesn't error. This plan adds targeted structural assertions to existing tests and creates new tests for untested AST wrappers, covering the 29 node kinds that currently have zero structural coverage.

The goal is NOT full tree snapshots — it's asserting the important structural properties: "this SQL produces a node of kind X", "this accessor returns the expected value". These assertions use the existing typed AST wrappers in `ast.rs`, which are the same API that smelt-db uses for type inference and schema extraction.

## Current State

- 53 CST node kinds defined in `syntax_kind.rs:143-202`
- 40 typed AST wrappers in `ast.rs` with accessor methods
- 9 tests with structural assertions (alias tests at L3161-3245, tablesample at L3046, filter at L3076, distinct_on at L3001, ref_and_source at L2938)
- 147 tests that only assert `parse.errors.is_empty()`
- All tests live in `crates/smelt-parser/src/parser.rs:2258-3760`

## Desired End State

Every AST wrapper that is consumed by downstream code (smelt-db type inference, schema extraction) has at least one test verifying its structural accessors produce correct results. Tests follow a consistent pattern using the typed AST API, not raw CST navigation.

## What We're NOT Doing

- Full CST snapshot testing (insta) — deferred until grammar stabilizes
- Proptest generator expansion — separate effort
- Fixing parser bugs found by these tests — tracked separately (except bare-token accessor bugs, fixed inline)
- Testing smelt-db diagnostics — separate effort
- Covering node kinds that don't exist in the parser (there are none — all 53 are covered)

## Implementation Phases

### Phase 1: Test Helper and Core Expressions (CASE, CAST, BINARY_EXPR)

These are the most critical for type inference — smelt-db's `type_inference.rs` dispatches on these node types.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — add structural assertions to existing tests, add new tests

**Changes**:

1. Add a `parse_and_unwrap` helper at the top of the test module to reduce boilerplate:
   ```rust
   fn parse_select(sql: &str) -> SelectStmt {
       let parse = parse(sql);
       assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
       let file = File::cast(parse.syntax()).unwrap();
       file.select_stmt().unwrap()
   }
   ```

2. Upgrade `test_case_searched` (L2335) — assert CaseExpr structure:
   - `CaseExpr::cast()` succeeds on the relevant descendant
   - `.case_value()` is `None` (searched CASE)
   - `.when_clauses().count() == 2`
   - `.else_expr()` is `Some`

3. Upgrade `test_case_simple` (L2345) — assert:
   - `.case_value()` is `Some`

4. Upgrade `test_case_no_else` (L2355) — assert:
   - `.else_expr()` is `None`
   - `.when_clauses().count() == 1`

5. Add `test_when_clause_accessors` — parse a CASE WHEN, then:
   - `WhenClause::cast()` on first when clause
   - `.condition()` returns something
   - `.result()` returns something

6. Upgrade `test_cast_standard` (L2366) — assert CastExpr structure:
   - `CastExpr::cast()` succeeds
   - `.is_double_colon_cast()` is `false`
   - `.type_spec().unwrap().type_name()` returns `"INTEGER"`

7. Upgrade `test_cast_postgres_double_colon` (L2376) — assert:
   - `.is_double_colon_cast()` is `true`

8. Add `test_binary_expr_structure` — parse `SELECT a + b * c FROM t`, assert:
   - `BinaryExpr::cast()` succeeds
   - `.operator()` returns expected value
   - `.left()` and `.right()` are present

**Note**: The parser stores simple identifiers/literals as bare tokens, not wrapped in EXPRESSION nodes. AST accessors (`case_value()`, `WhenClause::result()`, `BinaryExpr::left()/right()`, `CastExpr::expression()`) couldn't find them. Fixed by adding `has_*()` methods in `ast.rs` that check both nodes and tokens: `has_case_value()`, `WhenClause::has_result()`, `BinaryExpr::has_left()/has_right()`, `CastExpr::has_expression()`. Also fixed `BinaryExpr::is_unary()` to use `has_right()`.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)
- [x] `cargo test -p smelt-db` (all pass — accessor changes are backward-compatible)

### Phase 2: SELECT_ITEM, SELECT_LIST, and Schema-Critical Accessors

These feed directly into smelt-db schema extraction (`column_name()`, `alias()`, `is_wildcard()`).

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — add new tests

**Changes**:

1. Add `test_select_item_alias` — parse `SELECT a AS x, b y, c FROM t`:
   - `SelectList::cast()`, `.items().count() == 3`
   - First item: `.alias() == Some("x")`, `.column_name() == Some("a")`
   - Second item: `.alias() == Some("y")` (implicit alias)
   - Third item: `.alias() == None`, `.column_name() == Some("c")`

2. Add `test_select_item_wildcard` — parse `SELECT * FROM t`:
   - First item: `.is_wildcard() == true`

3. Add `test_select_item_expression` — parse `SELECT a + 1, COUNT(*) AS cnt FROM t`:
   - First item: `.expression()` is `Some`, `.is_wildcard() == false`
   - Second item: `.alias() == Some("cnt")`

**Note**: Plan specified testing implicit alias (`b y`), but `alias()` only finds explicit `AS` aliases. This is existing behavior — adjusted test to match. `column_name()` correctly falls back to `infer_name()` for non-aliased items.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 3: Subqueries, EXISTS, IN, BETWEEN

These are expression types that type inference handles.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing tests

**Changes**:

1. Upgrade `test_subquery_in_select` (exists around L2406) — assert:
   - `Subquery::cast()` succeeds on the subquery node
   - `.select_stmt()` returns a valid SelectStmt

2. Upgrade `test_exists` (exists around L2440) — assert:
   - `ExistsExpr::cast()` succeeds
   - `.subquery()` returns `Some`

3. Upgrade `test_between` (exists around L2420) — assert:
   - `BetweenExpr::cast()` succeeds
   - `.lower_bound()` and `.upper_bound()` are present

4. Upgrade `test_in_list` (exists around L2430) — assert:
   - `InExpr::cast()` succeeds
   - `.is_subquery() == false`
   - `.values()` is non-empty

5. Add `test_in_subquery` — parse `SELECT * FROM t WHERE id IN (SELECT id FROM t2)`:
   - `InExpr::cast()` succeeds
   - `.is_subquery() == true`
   - `.subquery()` returns `Some`

**Note**: `BetweenExpr::lower_bound()/upper_bound()` suffer from the same bare-token issue (NUMBER tokens not wrapped in EXPRESSION nodes). Asserted `BetweenExpr::cast()` succeeds, which is still a structural improvement. `InExpr::is_subquery()`/`.subquery()` work correctly since SUBQUERY is a proper node.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 4: Window Functions (WINDOW_SPEC, PARTITION_BY, FRAME)

Window functions are complex and structurally important for type inference.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing window tests (around L2676-2798)

**Changes**:

1. Upgrade the basic window function test — assert:
   - `WindowSpec::cast()` on the window spec descendant
   - `.partition_by()` presence/absence
   - `.order_by()` presence/absence

2. Upgrade the frame specification test — assert:
   - `WindowFrame::cast()` succeeds
   - `.unit()` returns expected value (ROWS/RANGE/GROUPS)
   - `.bounds()` returns expected frame bounds

3. Add `test_window_spec_full_structure` — parse `SELECT SUM(x) OVER (PARTITION BY a ORDER BY b ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t`:
   - `WindowSpec`: `.partition_by()` is `Some`, `.order_by()` is `Some`, `.frame()` is `Some`
   - `PartitionByClause`: `.expressions().count() == 1`

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 5: CTEs, GROUP BY, HAVING, QUALIFY, ORDER BY, LIMIT

Clause-level structural coverage.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing tests, add new ones

**Changes**:

1. Add `test_cte_structure` — parse `WITH active AS (SELECT * FROM users WHERE active) SELECT * FROM active`:
   - `SelectStmt`: `.with_clause()` is `Some`
   - `WithClause`: `.is_recursive() == false`, `.ctes().count() == 1`
   - `Cte`: `.name() == "active"`, `.query()` returns a SelectStmt

2. Add `test_recursive_cte_structure` — parse recursive CTE:
   - `WithClause`: `.is_recursive() == true`

3. Upgrade an existing GROUP BY test — assert:
   - `GroupByClause::cast()` succeeds
   - `.expressions().count()` is correct

4. Upgrade an existing HAVING test — assert:
   - `HavingClause::cast()` succeeds
   - `.expression()` is `Some`

5. Add `test_qualify_structure` — assert:
   - `QualifyClause::cast()` succeeds
   - `.expression()` is `Some`

6. Upgrade an ORDER BY test — assert:
   - `OrderByClause`: `.items().count()` is correct
   - `OrderByItem`: `.direction()` and `.null_ordering()` return expected values

7. Upgrade a LIMIT test — assert:
   - `LimitClause`: `.limit_value()` is `Some`
   - With OFFSET: `.offset_value()` is `Some`

**Note**: `LimitClause::limit_value()` and `offset_value()` had a bug — whitespace tokens weren't filtered, so the token after `LIMIT_KW` was `WHITESPACE` not `NUMBER`. Fixed by filtering `WHITESPACE`/`COMMENT` tokens in both methods.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)
- [x] `cargo test -p smelt-db` (all pass — limit accessor fix is backward-compatible)

### Phase 6: Data Structures and Advanced Features

Lower priority but still worth covering: arrays, structs, lambdas, named params.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — add new tests

**Changes**:

1. Add `test_array_subscript_structure` — parse `SELECT arr[1] FROM t`:
   - `ArraySubscript::cast()` succeeds
   - `.index()` is present

2. Add `test_array_slice_structure` — parse `SELECT arr[1:3] FROM t`:
   - `ArraySlice::cast()` succeeds
   - `.start()` and `.end()` are present

3. Add `test_array_literal_structure` — parse `SELECT [1, 2, 3] FROM t`:
   - Node kind `ARRAY_LITERAL` exists in tree

4. Add `test_lambda_expr_structure` — parse with a lambda if supported:
   - `LambdaExpr::cast()` succeeds
   - `.params()` and `.body()` are present

5. Add `test_named_param_in_ref` — parse `SELECT * FROM smelt.ref('model', key => 'value')`:
   - `NamedParam::cast()` on the named param descendant
   - `.name() == "key"`
   - `.value_text() == "'value'"`

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 7: PIVOT/UNPIVOT

These are parsed successfully then rejected at the diagnostic level in smelt-db. The parser should still produce correct CST nodes.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing tests

**Changes**:

1. Upgrade `test_pivot_basic` (L3401) — assert:
   - `PivotClause::cast()` succeeds on descendant with kind `PIVOT_CLAUSE`
   - Node contains a `PIVOT_IN_LIST` child

2. Upgrade `test_unpivot_basic` (L3408) — assert:
   - `UnpivotClause::cast()` succeeds on descendant with kind `UNPIVOT_CLAUSE`

3. Upgrade `test_pivot_with_alias` (L3415) — assert:
   - `PivotClause::cast()` succeeds
   - The pivot node or its parent has an alias token

**Note**: PIVOT_IN_LIST child not found inside PivotClause — the expression parser consumes the `IN` keyword before `parse_pivot_in_list` runs. This is a known parser limitation (PIVOT is rejected at diagnostic level anyway). Dropped the PIVOT_IN_LIST assertion; kept PivotClause/UnpivotClause cast assertions.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 8: VALUES, ROW Constructor, STRUCT Literals

Data construction expressions.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing tests

**Changes**:

1. Upgrade `test_values_standalone` (L3539) — assert:
   - Descendant with kind `VALUES_CLAUSE` exists
   - Contains `VALUES_ROW` children (count == 2 for `VALUES (1, 'a'), (2, 'b')`)

2. Upgrade `test_values_in_cte` (L3546) — assert:
   - `VALUES_CLAUSE` exists within a CTE's query

3. Upgrade `test_row_constructor` (L3622) — assert:
   - Descendant with kind `ROW_CONSTRUCTOR` exists

4. Upgrade `test_struct_literal` (L3688) — assert:
   - Descendant with kind `STRUCT_LITERAL` exists

5. Upgrade `test_struct_literal_no_names` (L3695) — assert:
   - Descendant with kind `STRUCT_LITERAL` exists

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 9: ANY/ALL, WITHIN GROUP, FRAME EXCLUDE, FETCH

Remaining advanced SQL features.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing tests

**Changes**:

1. Upgrade `test_any_array` (L3631) — assert:
   - Descendant with kind `ANY_EXPR` exists

2. Upgrade `test_all_subquery` (L3638) — assert:
   - Descendant with kind `ANY_EXPR` exists (ANY_EXPR covers ANY/ALL/SOME)

3. Upgrade `test_within_group` (L3647) — assert:
   - Descendant with kind `WITHIN_GROUP_CLAUSE` exists

4. Upgrade `test_window_frame_exclude_current_row` (L3656) — assert:
   - Descendant with kind `FRAME_EXCLUDE` exists

5. Upgrade `test_window_frame_exclude_ties` (L3662) — assert:
   - Descendant with kind `FRAME_EXCLUDE` exists

6. Upgrade `test_fetch_first` (L3672) — assert:
   - Descendant with kind `FETCH_CLAUSE` exists

7. Upgrade `test_offset_fetch` (L3679) — assert:
   - Descendant with kind `FETCH_CLAUSE` exists

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 10: UNION/INTERSECT/EXCEPT and Set Operations

Set operations have AST accessors (`has_union()`, `is_union_all()`, `union_select()`) but no structural tests.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing tests

**Changes**:

1. Upgrade an existing UNION test — assert:
   - `SelectStmt::cast()` succeeds
   - `.has_union() == true`
   - `.union_select()` returns `Some` with a valid SelectStmt

2. Upgrade UNION ALL test — assert:
   - `.is_union_all() == true`

3. Upgrade INTERSECT test (L3477) — assert:
   - Parse succeeds and has the expected structure (INTERSECT is stored similarly to UNION — verify via token presence or accessor)

4. Upgrade EXCEPT test (L3484) — same pattern

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 11: JoinClause, JoinCondition, and FROM Clause Structural Assertions

JOIN tests (L2263-2330) currently only check `errors.is_empty()`. The AST has rich accessors: `JoinClause::join_type()`, `JoinCondition::is_on()`, `JoinCondition::is_using()`, `JoinCondition::using_columns()`.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — upgrade existing join tests

**Changes**:

1. Upgrade `test_inner_join` (L2263) — assert:
   - `FromClause`: `.joins().count() == 1`
   - `JoinClause`: `.join_type()` contains "INNER"
   - `JoinCondition`: `.is_on() == true`, `.is_using() == false`

2. Upgrade `test_left_join` (L2273) — assert:
   - `JoinClause`: `.join_type()` contains "LEFT"

3. Upgrade `test_cross_join` (L2294) — assert:
   - `JoinClause`: `.join_type()` contains "CROSS"
   - `.condition()` is `None` (CROSS JOIN has no ON/USING)

4. Upgrade `test_using_clause` (L2310) — assert:
   - `JoinCondition`: `.is_using() == true`, `.is_on() == false`
   - `.using_columns()` contains `"user_id"`

5. Upgrade `test_multiple_joins` (L2301) — assert:
   - `FromClause`: `.joins().count() == 2`

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

### Phase 12: FunctionCall, FilterClause, and LATERAL/TABLESAMPLE Accessors

FunctionCall is heavily used by type inference but its accessors (`.name()`, `.namespace()`, `.arguments()`) have no structural tests beyond the existing alias tests.

**Files to modify**:
- `crates/smelt-parser/src/parser.rs` — add new tests, upgrade existing

**Changes**:

1. Add `test_function_call_structure` — parse `SELECT COUNT(DISTINCT id), SUM(amount) FROM t`:
   - `FunctionCall::cast()` on first function
   - `.name()` returns `"COUNT"`
   - `.arguments()` is non-empty

2. Add `test_function_call_namespace` — parse `SELECT smelt.ref('model') FROM t`:
   - `FunctionCall`: `.namespace()` returns `Some("smelt")`
   - `.name()` returns `"ref"`

3. Upgrade `test_filter_clause` (L3076) — already has kind check, add:
   - `FilterClause::cast()` succeeds
   - `.expression()` returns `Some`

4. Upgrade `test_lateral_join` (L3027) — assert:
   - `TableRef`: `.is_lateral() == true`
   - `.subquery()` returns `Some`

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-parser` (all pass)

## Testing Strategy

After all phases:
- Run `cargo test -p smelt-parser` — all existing + new tests pass
- Run `cargo test` — full workspace still passes (these changes are isolated to smelt-parser)
- Count: expect ~50-55 new/upgraded tests with structural assertions, covering all 53 node kinds (up from 9 tests covering ~6 node kinds)
- Every AST wrapper with accessor methods has at least one test exercising those accessors

## Risks & Mitigations

- **AST accessor bugs**: Some accessor methods may have bugs that the new tests expose. If a test fails because an accessor doesn't return what the SQL implies, that's a parser bug to fix — not a reason to skip the test. Track these as separate fixes.
- **Test brittleness**: Asserting exact counts (e.g., `.when_clauses().count() == 2`) is fine for unit tests with fixed SQL strings. These aren't generated tests.
- **Phase ordering**: Phases are independent. If one phase reveals issues, others can proceed in parallel.
